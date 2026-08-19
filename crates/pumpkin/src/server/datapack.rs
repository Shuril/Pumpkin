//! Transactional loading of world datapacks.
//!
//! The loader builds a complete candidate snapshot before it is published to
//! the live recipe manager. A malformed recipe therefore cannot leave half of
//! a reload visible to players. Both directory and zip datapacks are read
//! without mutating the live recipe registry until validation succeeds.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use pumpkin_data::item::Item;
use pumpkin_data::recipes::RecipeCategoryTypes;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::codec::recipe::{
    DynamicRecipe, OwnedCookingRecipe, OwnedCookingRecipeType, OwnedCraftingRecipe,
    OwnedRecipeIngredient, OwnedRecipeResult,
};
use pumpkin_util::identifier::Identifier;
use serde_json::Value;
use thiserror::Error;

use super::recipe::canonicalize_recipe_id;

#[derive(Debug, Error)]
pub enum DataPackError {
    #[error("failed to read datapack path {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid archive datapack {path}: {message}")]
    InvalidArchive { path: PathBuf, message: String },
    #[error("datapack {pack} has no pack.mcmeta")]
    MissingMetadata { pack: String },
    #[error("invalid pack.mcmeta in {path}: {message}")]
    InvalidMetadata { path: PathBuf, message: String },
    #[error("invalid recipe {path}: {message}")]
    InvalidRecipe { path: PathBuf, message: String },
    #[error("invalid tag {path}: {message}")]
    InvalidTag { path: PathBuf, message: String },
    #[error("invalid function {path}: {message}")]
    InvalidFunction { path: PathBuf, message: String },
    #[error("invalid {kind} resource {path}: {message}")]
    InvalidResource {
        kind: String,
        path: PathBuf,
        message: String,
    },
    #[error("enabled datapack {0:?} was not found")]
    MissingEnabledPack(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedDataPack {
    pub name: String,
    pub path: PathBuf,
    pub pack_format: i64,
}

#[derive(Clone, Debug, Default)]
pub struct DataPackSnapshot {
    pub packs: Vec<LoadedDataPack>,
    pub recipes: Vec<DynamicRecipe>,
    pub tags: TagSnapshot,
    pub functions: BTreeMap<String, Vec<String>>,
    /// Parsed JSON resources that are not recipes/tags/functions yet. Keeping
    /// them in the same immutable candidate makes `/reload` atomic for every
    /// resource family instead of publishing only the gameplay subset that
    /// currently has a consumer.
    pub resources: DataPackResources,
    pub unsupported_recipe_types: Vec<(String, String)>,
}

/// Forward-compatible datapack resources loaded from the active pack stack.
/// JSON values deliberately retain unknown fields so newer vanilla resources
/// survive a reload until their typed gameplay consumer is available. Structure
/// files are validated NBT and retained as their original bytes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DataPackResources {
    pub loot_tables: BTreeMap<String, Value>,
    pub predicates: BTreeMap<String, Value>,
    pub advancements: BTreeMap<String, Value>,
    pub structures: BTreeMap<String, Vec<u8>>,
}

/// A resource-pack tag entry.  Tags are kept as references until a caller
/// asks for membership, which preserves vanilla tag composition semantics and
/// lets a reload validate references before publishing the snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagValue {
    Element { id: String, required: bool },
    Tag { id: String, required: bool },
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TagDefinition {
    pub values: Vec<TagValue>,
    pub replace: bool,
}

/// Immutable, fully parsed datapack tag registry.  Keys are canonical tag
/// identifiers whose path is `<registry>/<tag path>`, for example
/// `minecraft:items/planks`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TagSnapshot {
    pub tags: BTreeMap<String, TagDefinition>,
}

impl TagSnapshot {
    /// Returns all resolved element IDs in deterministic order. Optional
    /// missing entries are ignored exactly as in the vanilla tag loader.
    #[must_use]
    pub fn members(&self, tag_id: &str) -> BTreeSet<String> {
        let Ok(tag_id) = canonical_tag_id(tag_id) else {
            return BTreeSet::new();
        };
        let mut result = BTreeSet::new();
        let mut visiting = HashSet::new();
        self.collect_members(&tag_id, &mut visiting, &mut result);
        result
    }

    #[must_use]
    pub fn contains(&self, tag_id: &str, element_id: &str) -> bool {
        let Ok(element_id) = canonical_element_id(element_id) else {
            return false;
        };
        self.members(tag_id).contains(&element_id)
    }

    /// Resolves a tag while preserving the declaration order used by the
    /// datapack.  This is intentionally separate from [`Self::members`],
    /// whose set semantics are useful for registry lookups but would change
    /// the observable order of `minecraft:tick`/`minecraft:load` functions.
    #[must_use]
    pub fn ordered_members(&self, tag_id: &str) -> Vec<String> {
        let Ok(tag_id) = canonical_tag_id(tag_id) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        let mut visiting = HashSet::new();
        let mut seen = HashSet::new();
        self.collect_ordered_members(&tag_id, &mut visiting, &mut seen, &mut result);
        result
    }

    fn collect_members(
        &self,
        tag_id: &str,
        visiting: &mut HashSet<String>,
        result: &mut BTreeSet<String>,
    ) {
        if !visiting.insert(tag_id.to_owned()) {
            return;
        }
        if let Some(definition) = self.tags.get(tag_id) {
            let registry = tag_registry(tag_id);
            for value in &definition.values {
                match value {
                    TagValue::Element { id, .. } => {
                        if !is_known_vanilla_element(registry, id) {
                            result.insert(id.clone());
                        } else if known_element_exists(registry, id) {
                            result.insert(id.clone());
                        }
                    }
                    TagValue::Tag { id, .. } => self.collect_members(id, visiting, result),
                }
            }
        }
        visiting.remove(tag_id);
    }

    fn collect_ordered_members(
        &self,
        tag_id: &str,
        visiting: &mut HashSet<String>,
        seen: &mut HashSet<String>,
        result: &mut Vec<String>,
    ) {
        if !visiting.insert(tag_id.to_owned()) {
            return;
        }
        if let Some(definition) = self.tags.get(tag_id) {
            for value in &definition.values {
                match value {
                    TagValue::Element { id, .. } => {
                        if seen.insert(id.clone()) {
                            result.push(id.clone());
                        }
                    }
                    TagValue::Tag { id, .. } => {
                        self.collect_ordered_members(id, visiting, seen, result);
                    }
                }
            }
        }
        visiting.remove(tag_id);
    }
}

pub struct DataPackLoader;

impl DataPackLoader {
    /// Loads enabled directory datapacks in persisted priority order.
    pub fn load(world_path: &Path, enabled: &[String]) -> Result<DataPackSnapshot, DataPackError> {
        let root = world_path.join("datapacks");
        if !root.exists() {
            // The built-in vanilla pack is implicit, but an explicitly
            // enabled file/ pack must never disappear silently.  Vanilla
            // aborts the reload and keeps the previous snapshot in this
            // situation; returning an empty candidate would instead delete
            // every active recipe/function/tag on `/reload`.
            if let Some(missing) = enabled.iter().find(|name| name.as_str() != "vanilla") {
                return Err(DataPackError::MissingEnabledPack(missing.clone()));
            }
            return Ok(DataPackSnapshot::default());
        }

        let mut available = BTreeMap::<String, PathBuf>::new();
        for entry in fs::read_dir(&root).map_err(|source| DataPackError::Io {
            path: root.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| DataPackError::Io {
                path: root.clone(),
                source,
            })?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                available.insert(name, path);
            } else if path.extension().is_some_and(|extension| extension == "zip") {
                available.insert(name, path);
            }
        }

        let mut snapshot = DataPackSnapshot::default();
        let mut recipes = BTreeMap::<String, DynamicRecipe>::new();
        let mut tags = BTreeMap::<String, TagDefinition>::new();
        let mut functions = BTreeMap::<String, Vec<String>>::new();
        let mut resources = DataPackResources::default();
        for persisted_name in enabled {
            if persisted_name == "vanilla" {
                continue;
            }
            let name = persisted_name
                .strip_prefix("file/")
                .unwrap_or(persisted_name.as_str());
            let Some(path) = available.get(name) else {
                return Err(DataPackError::MissingEnabledPack(persisted_name.clone()));
            };
            if path.is_dir() {
                let pack = load_metadata(name, path)?;
                let mut pack_recipes = Vec::new();
                collect_recipe_files(&path.join("data"), &mut pack_recipes)?;
                for recipe_path in pack_recipes {
                    match parse_recipe_file(&recipe_path) {
                        Ok((id, recipe)) => {
                            recipes.insert(id, recipe);
                        }
                        Err(DataPackError::InvalidRecipe { path, message })
                            if message.starts_with("unsupported recipe type ") =>
                        {
                            snapshot
                                .unsupported_recipe_types
                                .push((path.display().to_string(), message));
                        }
                        Err(error) => return Err(error),
                    }
                }
                let mut pack_tags = Vec::new();
                collect_tag_files(&path.join("data"), &mut pack_tags)?;
                for tag_path in pack_tags {
                    let (id, definition) = parse_tag_file(&tag_path)?;
                    merge_tag(&mut tags, id, definition);
                }
                let mut pack_functions = Vec::new();
                collect_function_files(&path.join("data"), &mut pack_functions)?;
                for function_path in pack_functions {
                    let (id, commands) = parse_function_file(&function_path)?;
                    functions.insert(id, commands);
                }
                load_directory_resources(&path.join("data"), &mut resources)?;
                snapshot.packs.push(pack);
            } else {
                let pack = load_zip_pack(
                    name,
                    path,
                    &mut recipes,
                    &mut tags,
                    &mut functions,
                    &mut resources,
                    &mut snapshot,
                )?;
                snapshot.packs.push(pack);
            }
        }
        snapshot.recipes = recipes.into_values().collect();
        snapshot.tags = TagSnapshot { tags };
        snapshot.functions = functions;
        snapshot.resources = resources;
        validate_tag_snapshot(&snapshot.tags)?;
        validate_function_tag_references(&snapshot.tags, &snapshot.functions)?;
        Ok(snapshot)
    }
}

fn load_metadata(name: &str, path: &Path) -> Result<LoadedDataPack, DataPackError> {
    let metadata_path = path.join("pack.mcmeta");
    if !metadata_path.exists() {
        return Err(DataPackError::MissingMetadata {
            pack: name.to_owned(),
        });
    }
    let text = fs::read_to_string(&metadata_path).map_err(|source| DataPackError::Io {
        path: metadata_path.clone(),
        source,
    })?;
    let value: Value =
        serde_json::from_str(&text).map_err(|error| DataPackError::InvalidMetadata {
            path: metadata_path.clone(),
            message: error.to_string(),
        })?;
    let pack = value
        .get("pack")
        .and_then(Value::as_object)
        .ok_or_else(|| DataPackError::InvalidMetadata {
            path: metadata_path.clone(),
            message: "missing pack object".to_owned(),
        })?;
    let pack_format = pack
        .get("pack_format")
        .and_then(Value::as_i64)
        .ok_or_else(|| DataPackError::InvalidMetadata {
            path: metadata_path,
            message: "pack.pack_format must be an integer".to_owned(),
        })?;
    if pack_format < 1 {
        return Err(DataPackError::InvalidMetadata {
            path: path.join("pack.mcmeta"),
            message: "pack.pack_format must be positive".to_owned(),
        });
    }
    Ok(LoadedDataPack {
        name: name.to_owned(),
        path: path.to_path_buf(),
        pack_format,
    })
}

fn load_zip_pack(
    name: &str,
    path: &Path,
    recipes: &mut BTreeMap<String, DynamicRecipe>,
    tags: &mut BTreeMap<String, TagDefinition>,
    functions: &mut BTreeMap<String, Vec<String>>,
    resources: &mut DataPackResources,
    snapshot: &mut DataPackSnapshot,
) -> Result<LoadedDataPack, DataPackError> {
    let file = File::open(path).map_err(|source| DataPackError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|error| DataPackError::InvalidArchive {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    let metadata = {
        let mut entry =
            archive
                .by_name("pack.mcmeta")
                .map_err(|error| DataPackError::InvalidArchive {
                    path: path.to_path_buf(),
                    message: format!("missing pack.mcmeta: {error}"),
                })?;
        let mut text = String::new();
        std::io::Read::read_to_string(&mut entry, &mut text).map_err(|error| {
            DataPackError::InvalidArchive {
                path: path.to_path_buf(),
                message: format!("cannot read pack.mcmeta: {error}"),
            }
        })?;
        parse_metadata_value(name, path, &text)?
    };

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| DataPackError::InvalidArchive {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
        let entry_name = entry.name().replace('\\', "/");
        let is_function = entry_name.starts_with("data/")
            && entry_name.contains("/function/")
            && entry_name.ends_with(".mcfunction");
        let is_json_resource = entry_name.ends_with(".json")
            && (entry_name.contains("/recipes/")
                || entry_name.contains("/tags/")
                || entry_name.contains("/loot_tables/")
                || entry_name.contains("/predicates/")
                || entry_name.contains("/advancements/"));
        let is_structure_resource =
            entry_name.ends_with(".nbt") && entry_name.contains("/structures/");
        if !is_safe_archive_path(&entry_name)
            || !entry_name.starts_with("data/")
            || (!is_function && !is_json_resource && !is_structure_resource)
        {
            continue;
        }
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).map_err(|error| {
            DataPackError::InvalidArchive {
                path: path.to_path_buf(),
                message: format!("cannot read {entry_name}: {error}"),
            }
        })?;
        let resource_path = path.join(&entry_name);
        if is_function {
            let text =
                String::from_utf8(bytes).map_err(|error| DataPackError::InvalidFunction {
                    path: resource_path.clone(),
                    message: format!("function is not valid UTF-8: {error}"),
                })?;
            let id = function_id_from_resource_path(&entry_name, &resource_path)?;
            functions.insert(id, parse_function_text(&resource_path, &text)?);
        } else if is_structure_resource {
            let (kind, id) = resource_id_from_resource_path(&entry_name, &resource_path)?;
            debug_assert_eq!(kind, "structures");
            validate_structure_bytes(&resource_path, &bytes)?;
            resources.structures.insert(id, bytes);
        } else {
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|error| DataPackError::InvalidArchive {
                    path: path.to_path_buf(),
                    message: format!("invalid JSON in {entry_name}: {error}"),
                })?;
            if entry_name.contains("/recipes/") {
                let id = recipe_id_from_resource_path(&entry_name, &resource_path)?;
                match parse_recipe_value(&resource_path, &value, &id) {
                    Ok(recipe) => {
                        recipes.insert(id, recipe);
                    }
                    Err(DataPackError::InvalidRecipe { path, message })
                        if message.starts_with("unsupported recipe type ") =>
                    {
                        snapshot
                            .unsupported_recipe_types
                            .push((path.display().to_string(), message));
                    }
                    Err(error) => return Err(error),
                }
            } else if entry_name.contains("/tags/") {
                let id = tag_id_from_resource_path(&entry_name, &resource_path)?;
                let definition = parse_tag_value(&resource_path, &value)?;
                merge_tag(tags, id, definition);
            } else {
                let (kind, id) = resource_id_from_resource_path(&entry_name, &resource_path)?;
                let parsed = parse_json_resource(&resource_path, &kind, value)?;
                insert_json_resource(resources, kind.as_str(), id, parsed);
            }
        }
    }
    Ok(metadata)
}

fn parse_metadata_value(
    name: &str,
    path: &Path,
    text: &str,
) -> Result<LoadedDataPack, DataPackError> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| DataPackError::InvalidMetadata {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    let pack = value
        .get("pack")
        .and_then(Value::as_object)
        .ok_or_else(|| DataPackError::InvalidMetadata {
            path: path.to_path_buf(),
            message: "missing pack object".to_owned(),
        })?;
    let pack_format = pack
        .get("pack_format")
        .and_then(Value::as_i64)
        .ok_or_else(|| DataPackError::InvalidMetadata {
            path: path.to_path_buf(),
            message: "pack.pack_format must be an integer".to_owned(),
        })?;
    if pack_format < 1 {
        return Err(DataPackError::InvalidMetadata {
            path: path.to_path_buf(),
            message: "pack.pack_format must be positive".to_owned(),
        });
    }
    Ok(LoadedDataPack {
        name: name.to_owned(),
        path: path.to_path_buf(),
        pack_format,
    })
}

fn is_safe_archive_path(path: &str) -> bool {
    !path.starts_with('/')
        && !path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
}

fn load_directory_resources(
    data_root: &Path,
    resources: &mut DataPackResources,
) -> Result<(), DataPackError> {
    for (directory, kind) in [
        ("loot_tables", "loot_tables"),
        ("predicates", "predicates"),
        ("advancements", "advancements"),
    ] {
        let mut files = Vec::new();
        collect_resource_files(data_root, directory, "json", &mut files)?;
        for path in files {
            let id = resource_id_from_path(&path, directory, "json")?;
            let text = fs::read_to_string(&path).map_err(|source| DataPackError::Io {
                path: path.clone(),
                source,
            })?;
            let value: Value = serde_json::from_str(&text)
                .map_err(|error| invalid_resource(&path, kind, error.to_string()))?;
            let value = parse_json_resource(&path, kind, value)?;
            insert_json_resource(resources, kind, id, value);
        }
    }

    let mut structure_files = Vec::new();
    collect_resource_files(data_root, "structures", "nbt", &mut structure_files)?;
    for path in structure_files {
        let id = resource_id_from_path(&path, "structures", "nbt")?;
        let bytes = fs::read(&path).map_err(|source| DataPackError::Io {
            path: path.clone(),
            source,
        })?;
        validate_structure_bytes(&path, &bytes)?;
        resources.structures.insert(id, bytes);
    }
    Ok(())
}

fn collect_resource_files(
    root: &Path,
    directory: &str,
    extension: &str,
    output: &mut Vec<PathBuf>,
) -> Result<(), DataPackError> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|source| DataPackError::Io {
        path: root.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| DataPackError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_resource_files(&path, directory, extension, output)?;
        } else if path.extension().is_some_and(|value| value == extension)
            && path
                .components()
                .any(|component| component.as_os_str() == directory)
        {
            output.push(path);
        }
    }
    output.sort();
    Ok(())
}

fn resource_id_from_path(
    path: &Path,
    directory: &str,
    extension: &str,
) -> Result<String, DataPackError> {
    let components = path
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid_resource(path, directory, "resource path is not UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let data_index = components
        .iter()
        .rposition(|component| component == "data")
        .ok_or_else(|| invalid_resource(path, directory, "resource is outside data/"))?;
    let namespace = components
        .get(data_index + 1)
        .ok_or_else(|| invalid_resource(path, directory, "resource namespace is missing"))?;
    let directory_index = components
        .iter()
        .enumerate()
        .skip(data_index + 2)
        .find_map(|(index, component)| (component == directory).then_some(index))
        .ok_or_else(|| invalid_resource(path, directory, "resource directory is missing"))?;
    let resource_parts = &components[directory_index + 1..];
    let Some(last) = resource_parts.last() else {
        return Err(invalid_resource(
            path,
            directory,
            "resource name is missing",
        ));
    };
    let suffix = format!(".{extension}");
    if !last.ends_with(&suffix) {
        return Err(invalid_resource(
            path,
            directory,
            "invalid resource extension",
        ));
    }
    let mut resource = resource_parts.to_vec();
    let last_index = resource.len() - 1;
    resource[last_index] = last.trim_end_matches(&suffix).to_owned();
    Identifier::new(namespace.to_owned(), resource.join("/"))
        .map(|identifier| identifier.to_string())
        .map_err(|error| invalid_resource(path, directory, error.to_string()))
}

fn resource_id_from_resource_path(
    resource_path: &str,
    path: &Path,
) -> Result<(String, String), DataPackError> {
    let components = resource_path.split('/').collect::<Vec<_>>();
    if components.len() < 4 || components[0] != "data" {
        return Err(invalid_resource(
            path,
            "resource",
            "resource is outside data/",
        ));
    }
    let directory = components[2];
    if !matches!(
        directory,
        "loot_tables" | "predicates" | "advancements" | "structures"
    ) {
        return Err(invalid_resource(
            path,
            "resource",
            "unknown resource directory",
        ));
    }
    let directory_index = 2;
    let resource_parts = &components[directory_index + 1..];
    let Some(last) = resource_parts.last() else {
        return Err(invalid_resource(
            path,
            directory,
            "resource name is missing",
        ));
    };
    let extension = if directory == "structures" {
        ".nbt"
    } else {
        ".json"
    };
    if !last.ends_with(extension) {
        return Err(invalid_resource(
            path,
            directory,
            "invalid resource extension",
        ));
    }
    let mut resource = resource_parts.to_vec();
    let last_index = resource.len() - 1;
    resource[last_index] = last.trim_end_matches(extension);
    let id = Identifier::new(components[1].to_owned(), resource.join("/"))
        .map_err(|error| invalid_resource(path, directory, error.to_string()))?
        .to_string();
    Ok((directory.to_owned(), id))
}

fn parse_json_resource(path: &Path, kind: &str, value: Value) -> Result<Value, DataPackError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid_resource(path, kind, "resource root must be an object"))?;
    match kind {
        "loot_tables" => {
            if let Some(pools) = object.get("pools")
                && !pools.is_array()
            {
                return Err(invalid_resource(path, kind, "pools must be an array"));
            }
            if let Some(value) = object.get("type")
                && !value.is_string()
            {
                return Err(invalid_resource(path, kind, "type must be a string"));
            }
        }
        "predicates" => {
            if let Some(condition) = object.get("condition")
                && !condition.is_string()
            {
                return Err(invalid_resource(path, kind, "condition must be a string"));
            }
        }
        "advancements" => {
            let criteria = object
                .get("criteria")
                .ok_or_else(|| invalid_resource(path, kind, "criteria is required"))?;
            if !criteria.is_object() {
                return Err(invalid_resource(path, kind, "criteria must be an object"));
            }
            if let Some(parent) = object.get("parent")
                && !parent.is_string()
            {
                return Err(invalid_resource(path, kind, "parent must be a string"));
            }
        }
        _ => return Err(invalid_resource(path, kind, "unknown JSON resource kind")),
    }
    Ok(value)
}

fn insert_json_resource(resources: &mut DataPackResources, kind: &str, id: String, value: Value) {
    match kind {
        "loot_tables" => {
            resources.loot_tables.insert(id, value);
        }
        "predicates" => {
            resources.predicates.insert(id, value);
        }
        "advancements" => {
            resources.advancements.insert(id, value);
        }
        _ => {}
    }
}

fn validate_structure_bytes(path: &Path, bytes: &[u8]) -> Result<(), DataPackError> {
    const MAX_STRUCTURE_BYTES: usize = 16 * 1024 * 1024;
    if bytes.is_empty() || bytes.len() > MAX_STRUCTURE_BYTES {
        return Err(invalid_resource(
            path,
            "structures",
            format!("NBT size must be between 1 and {MAX_STRUCTURE_BYTES} bytes"),
        ));
    }
    let plain = pumpkin_nbt::Nbt::read(&mut pumpkin_nbt::deserializer::NbtReadHelperJava::new(
        Cursor::new(bytes),
    ));
    if plain.is_ok() {
        return Ok(());
    }
    if pumpkin_nbt::nbt_compress::read_gzip_compound_tag(Cursor::new(bytes)).is_ok() {
        return Ok(());
    }
    Err(invalid_resource(
        path,
        "structures",
        format!("invalid named or gzip-compressed NBT: {plain:?}"),
    ))
}

fn invalid_resource(
    path: impl Into<PathBuf>,
    kind: impl Into<String>,
    message: impl Into<String>,
) -> DataPackError {
    DataPackError::InvalidResource {
        kind: kind.into(),
        path: path.into(),
        message: message.into(),
    }
}

fn collect_recipe_files(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), DataPackError> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|source| DataPackError::Io {
        path: root.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| DataPackError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_recipe_files(&path, output)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "json")
            && path
                .components()
                .any(|component| component.as_os_str() == "recipes")
        {
            output.push(path);
        }
    }
    output.sort();
    Ok(())
}

fn collect_tag_files(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), DataPackError> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|source| DataPackError::Io {
        path: root.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| DataPackError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_tag_files(&path, output)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "json")
            && path
                .components()
                .any(|component| component.as_os_str() == "tags")
        {
            output.push(path);
        }
    }
    output.sort();
    Ok(())
}

fn collect_function_files(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), DataPackError> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|source| DataPackError::Io {
        path: root.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| DataPackError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_function_files(&path, output)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "mcfunction")
            && path
                .components()
                .any(|component| component.as_os_str() == "function")
        {
            output.push(path);
        }
    }
    output.sort();
    Ok(())
}

fn parse_function_file(path: &Path) -> Result<(String, Vec<String>), DataPackError> {
    let text = fs::read_to_string(path).map_err(|source| DataPackError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let id = function_id_from_path(path)?;
    Ok((id, parse_function_text(path, &text)?))
}

fn parse_function_text(path: &Path, text: &str) -> Result<Vec<String>, DataPackError> {
    const MAX_FUNCTION_LINES: usize = 65_536;
    const MAX_COMMAND_LENGTH: usize = 32_767;
    let mut commands = Vec::new();
    for raw_line in text.strip_prefix('\u{feff}').unwrap_or(text).lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.contains('\0') {
            return Err(invalid_function(path, "command contains a NUL byte"));
        }
        if line.len() > MAX_COMMAND_LENGTH {
            return Err(invalid_function(
                path,
                format!("command exceeds {MAX_COMMAND_LENGTH} bytes"),
            ));
        }
        commands.push(line.to_owned());
        if commands.len() > MAX_FUNCTION_LINES {
            return Err(invalid_function(
                path,
                format!("function exceeds {MAX_FUNCTION_LINES} commands"),
            ));
        }
    }
    Ok(commands)
}

fn merge_tag(tags: &mut BTreeMap<String, TagDefinition>, id: String, incoming: TagDefinition) {
    if incoming.replace {
        tags.insert(
            id,
            TagDefinition {
                values: incoming.values,
                replace: false,
            },
        );
        return;
    }
    let entry = tags.entry(id).or_default();
    let mut seen = entry
        .values
        .iter()
        .map(tag_value_key)
        .collect::<HashSet<_>>();
    for value in incoming.values {
        if seen.insert(tag_value_key(&value)) {
            entry.values.push(value);
        }
    }
}

fn parse_tag_file(path: &Path) -> Result<(String, TagDefinition), DataPackError> {
    let text = fs::read_to_string(path).map_err(|source| DataPackError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|error| DataPackError::InvalidTag {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let id = tag_id_from_path(path)?;
    Ok((id, parse_tag_value(path, &value)?))
}

fn parse_tag_value(path: &Path, value: &Value) -> Result<TagDefinition, DataPackError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid_tag(path, "tag must be an object"))?;
    let replace = object
        .get("replace")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let values = object
        .get("values")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_tag(path, "tag.values must be an array"))?
        .iter()
        .map(|value| parse_tag_entry(path, value))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TagDefinition { values, replace })
}

fn parse_tag_entry(path: &Path, value: &Value) -> Result<TagValue, DataPackError> {
    let (raw_id, required) = if let Some(id) = value.as_str() {
        (id, true)
    } else {
        let object = value
            .as_object()
            .ok_or_else(|| invalid_tag(path, "tag values must be strings or objects"))?;
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_tag(path, "tag value object requires string id"))?;
        let required = object
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        (id, required)
    };
    if raw_id.is_empty() || raw_id == "#" {
        return Err(invalid_tag(path, "tag value id cannot be empty"));
    }
    if let Some(tag) = raw_id.strip_prefix('#') {
        let id = canonical_tag_id(tag)
            .map_err(|message| invalid_tag(path, format!("invalid tag reference: {message}")))?;
        Ok(TagValue::Tag { id, required })
    } else {
        let id = canonical_element_id(raw_id)
            .map_err(|message| invalid_tag(path, format!("invalid element id: {message}")))?;
        Ok(TagValue::Element { id, required })
    }
}

fn tag_id_from_path(path: &Path) -> Result<String, DataPackError> {
    let components = path
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid_tag(path, "tag path is not valid UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let data_index = components
        .iter()
        .rposition(|component| component == "data")
        .ok_or_else(|| invalid_tag(path, "tag is outside a data namespace"))?;
    let namespace = components
        .get(data_index + 1)
        .ok_or_else(|| invalid_tag(path, "tag namespace is missing"))?;
    let tags_index = components
        .iter()
        .enumerate()
        .skip(data_index + 2)
        .find_map(|(index, component)| (component == "tags").then_some(index))
        .ok_or_else(|| invalid_tag(path, "tag is outside a tags directory"))?;
    let registry = components
        .get(tags_index + 1)
        .ok_or_else(|| invalid_tag(path, "tag registry is missing"))?;
    let resource_parts = &components[tags_index + 2..];
    let Some(last) = resource_parts.last() else {
        return Err(invalid_tag(path, "tag resource name is missing"));
    };
    if !last.ends_with(".json") || resource_parts.len() == 1 && last == "tags.json" {
        return Err(invalid_tag(path, "tag resource must end with .json"));
    }
    let mut resource = resource_parts.to_vec();
    let last_index = resource.len() - 1;
    resource[last_index] = last.trim_end_matches(".json").to_owned();
    canonical_tag_id(&format!("{namespace}:{}/{}", registry, resource.join("/")))
        .map_err(|message| invalid_tag(path, message))
}

fn tag_id_from_resource_path(resource_path: &str, path: &Path) -> Result<String, DataPackError> {
    let components = resource_path.split('/').collect::<Vec<_>>();
    if components.len() < 5 || components[0] != "data" || components[2] != "tags" {
        return Err(invalid_tag(
            path,
            "tag is outside a data namespace tags directory",
        ));
    }
    let resource_parts = &components[4..];
    let Some(last) = resource_parts.last() else {
        return Err(invalid_tag(path, "tag resource name is missing"));
    };
    if !last.ends_with(".json") {
        return Err(invalid_tag(path, "tag resource must end with .json"));
    }
    let mut resource = resource_parts.to_vec();
    let last_index = resource.len() - 1;
    resource[last_index] = last.trim_end_matches(".json");
    canonical_tag_id(&format!(
        "{}:{}/{}",
        components[1],
        components[3],
        resource.join("/")
    ))
    .map_err(|message| invalid_tag(path, message))
}

fn canonical_tag_id(id: &str) -> Result<String, String> {
    let id = id.strip_prefix('#').unwrap_or(id);
    let identifier = Identifier::parse(id).map_err(|error| error.to_string())?;
    let (namespace, path) = identifier.view();
    let Some((registry, tag_path)) = path.split_once('/') else {
        return Err("tag path must include a registry and resource path".to_owned());
    };
    if registry.is_empty() || tag_path.is_empty() {
        return Err("tag registry and resource path cannot be empty".to_owned());
    }
    Ok(format!("{namespace}:{path}"))
}

fn canonical_element_id(id: &str) -> Result<String, String> {
    Ok(Identifier::parse(id)
        .map_err(|error| error.to_string())?
        .to_string())
}

fn tag_registry(tag_id: &str) -> &str {
    tag_id
        .split_once(':')
        .and_then(|(_, path)| path.split_once('/'))
        .map_or("", |(registry, _)| registry)
}

fn is_known_vanilla_element(registry: &str, id: &str) -> bool {
    id.starts_with("minecraft:") && matches!(registry, "items" | "blocks" | "fluids")
}

fn known_element_exists(registry: &str, id: &str) -> bool {
    let key = id.strip_prefix("minecraft:").unwrap_or(id);
    match registry {
        "items" => pumpkin_data::item::Item::from_registry_key(key).is_some(),
        "blocks" => pumpkin_data::Block::from_registry_key(key).is_some(),
        "fluids" => pumpkin_data::fluid::Fluid::from_registry_key(key).is_some(),
        _ => true,
    }
}

fn tag_value_key(value: &TagValue) -> String {
    match value {
        TagValue::Element { id, required } => format!("element:{id}:{required}"),
        TagValue::Tag { id, required } => format!("tag:{id}:{required}"),
    }
}

fn validate_tag_snapshot(snapshot: &TagSnapshot) -> Result<(), DataPackError> {
    let mut visiting = HashSet::new();
    let mut validated = HashSet::new();
    for id in snapshot.tags.keys() {
        validate_tag(snapshot, id, &mut visiting, &mut validated)?;
    }
    Ok(())
}

fn validate_tag(
    snapshot: &TagSnapshot,
    id: &str,
    visiting: &mut HashSet<String>,
    validated: &mut HashSet<String>,
) -> Result<(), DataPackError> {
    if validated.contains(id) {
        return Ok(());
    }
    if !visiting.insert(id.to_owned()) {
        return Err(invalid_tag(PathBuf::from(id), "cyclic tag reference"));
    }
    let Some(definition) = snapshot.tags.get(id) else {
        visiting.remove(id);
        return Ok(());
    };
    let registry = tag_registry(id);
    for value in &definition.values {
        match value {
            TagValue::Element {
                id: element,
                required,
            } => {
                if *required
                    && is_known_vanilla_element(registry, element)
                    && !known_element_exists(registry, element)
                {
                    return Err(invalid_tag(
                        PathBuf::from(id),
                        format!("required element {element} does not exist"),
                    ));
                }
            }
            TagValue::Tag {
                id: referenced,
                required,
            } => {
                if tag_registry(referenced) != registry {
                    return Err(invalid_tag(
                        PathBuf::from(id),
                        format!("tag reference {referenced} uses a different registry"),
                    ));
                }
                if !snapshot.tags.contains_key(referenced) && *required {
                    return Err(invalid_tag(
                        PathBuf::from(id),
                        format!("required tag {referenced} does not exist"),
                    ));
                }
                if snapshot.tags.contains_key(referenced) {
                    validate_tag(snapshot, referenced, visiting, validated)?;
                }
            }
        }
    }
    visiting.remove(id);
    validated.insert(id.to_owned());
    Ok(())
}

fn validate_function_tag_references(
    snapshot: &TagSnapshot,
    functions: &BTreeMap<String, Vec<String>>,
) -> Result<(), DataPackError> {
    for (tag_id, definition) in &snapshot.tags {
        if tag_registry(tag_id) != "function" {
            continue;
        }
        for value in &definition.values {
            let TagValue::Element { id, required } = value else {
                continue;
            };
            if *required && !functions.contains_key(id) {
                return Err(invalid_tag(
                    PathBuf::from(tag_id),
                    format!("required function {id} does not exist"),
                ));
            }
        }
    }
    Ok(())
}

fn invalid_tag(path: impl Into<PathBuf>, message: impl Into<String>) -> DataPackError {
    DataPackError::InvalidTag {
        path: path.into(),
        message: message.into(),
    }
}

fn parse_recipe_file(path: &Path) -> Result<(String, DynamicRecipe), DataPackError> {
    let text = fs::read_to_string(path).map_err(|source| DataPackError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let value: Value =
        serde_json::from_str(&text).map_err(|error| DataPackError::InvalidRecipe {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    let id = recipe_id_from_path(path)?;
    let recipe = parse_recipe_value(path, &value, &id)?;
    Ok((id, recipe))
}

fn parse_recipe_value(
    path: &Path,
    value: &Value,
    id: &str,
) -> Result<DynamicRecipe, DataPackError> {
    let recipe_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_recipe(path, "missing string type"))?;
    let recipe = match recipe_type {
        "minecraft:crafting_shaped" => DynamicRecipe::Crafting(parse_shaped(path, &value, &id)?),
        "minecraft:crafting_shapeless" => {
            DynamicRecipe::Crafting(parse_shapeless(path, &value, &id)?)
        }
        "minecraft:smelting" => DynamicRecipe::Cooking(OwnedCookingRecipeType::Smelting(
            parse_cooking(path, &value, &id)?,
        )),
        "minecraft:blasting" => DynamicRecipe::Cooking(OwnedCookingRecipeType::Blasting(
            parse_cooking(path, &value, &id)?,
        )),
        "minecraft:smoking" => DynamicRecipe::Cooking(OwnedCookingRecipeType::Smoking(
            parse_cooking(path, &value, &id)?,
        )),
        "minecraft:campfire_cooking" => DynamicRecipe::Cooking(
            OwnedCookingRecipeType::CampfireCooking(parse_cooking(path, &value, &id)?),
        ),
        other => {
            return Err(invalid_recipe(
                path,
                format!("unsupported recipe type {other}"),
            ));
        }
    };
    Ok(recipe)
}

fn parse_shaped(
    path: &Path,
    value: &Value,
    id: &str,
) -> Result<OwnedCraftingRecipe, DataPackError> {
    let pattern = value
        .get("pattern")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_recipe(path, "crafting_shaped.pattern must be an array"))?
        .iter()
        .map(|row| {
            row.as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid_recipe(path, "crafting_shaped.pattern rows must be strings"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if pattern.is_empty()
        || pattern.len() > 3
        || pattern.iter().any(|row| row.is_empty() || row.len() > 3)
    {
        return Err(invalid_recipe(
            path,
            "crafting_shaped pattern must be 1..=3 by 1..=3",
        ));
    }
    let key = parse_key(path, value.get("key"))?;
    for row in &pattern {
        for character in row.chars().filter(|character| *character != ' ') {
            if !key.iter().any(|(key, _)| *key == character) {
                return Err(invalid_recipe(
                    path,
                    format!("pattern references unknown key {character:?}"),
                ));
            }
        }
    }
    Ok(OwnedCraftingRecipe::Shaped {
        recipe_id: id.to_owned(),
        category: parse_category(value),
        group: value
            .get("group")
            .and_then(Value::as_str)
            .map(str::to_owned),
        show_notification: value
            .get("show_notification")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        key,
        pattern,
        result: parse_result(path, value.get("result"))?,
    })
}

fn parse_shapeless(
    path: &Path,
    value: &Value,
    id: &str,
) -> Result<OwnedCraftingRecipe, DataPackError> {
    let ingredients = value
        .get("ingredients")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_recipe(path, "crafting_shapeless.ingredients must be an array"))?
        .iter()
        .map(|ingredient| parse_ingredient(path, ingredient))
        .collect::<Result<Vec<_>, _>>()?;
    if ingredients.is_empty() || ingredients.len() > 9 {
        return Err(invalid_recipe(
            path,
            "crafting_shapeless must contain 1..=9 ingredients",
        ));
    }
    Ok(OwnedCraftingRecipe::Shapeless {
        recipe_id: id.to_owned(),
        category: parse_category(value),
        group: value
            .get("group")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ingredients,
        result: parse_result(path, value.get("result"))?,
    })
}

fn parse_cooking(
    path: &Path,
    value: &Value,
    id: &str,
) -> Result<OwnedCookingRecipe, DataPackError> {
    let cooking_time = value
        .get("cookingtime")
        .or_else(|| value.get("cooking_time"))
        .and_then(Value::as_i64)
        .unwrap_or(200);
    let experience = value
        .get("experience")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if cooking_time < 1 || cooking_time > i32::MAX as i64 || !experience.is_finite() {
        return Err(invalid_recipe(path, "invalid cooking time or experience"));
    }
    let ingredient = value
        .get("ingredient")
        .ok_or_else(|| invalid_recipe(path, "missing ingredient"))?;
    Ok(OwnedCookingRecipe {
        recipe_id: id.to_owned(),
        category: parse_category(value),
        group: value
            .get("group")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ingredient: parse_ingredient(path, ingredient)?,
        cooking_time: cooking_time as i32,
        experience: experience as f32,
        result: parse_result(path, value.get("result"))?,
    })
}

fn parse_key(
    path: &Path,
    value: Option<&Value>,
) -> Result<Vec<(char, OwnedRecipeIngredient)>, DataPackError> {
    let object = value
        .and_then(Value::as_object)
        .ok_or_else(|| invalid_recipe(path, "crafting_shaped.key must be an object"))?;
    let mut key = Vec::with_capacity(object.len());
    for (character, ingredient) in object {
        let mut chars = character.chars();
        let Some(character) = chars.next() else {
            return Err(invalid_recipe(path, "crafting key cannot be empty"));
        };
        if chars.next().is_some() || character == ' ' {
            return Err(invalid_recipe(
                path,
                "crafting key entries must be one non-space character",
            ));
        }
        key.push((character, parse_ingredient(path, ingredient)?));
    }
    Ok(key)
}

fn parse_ingredient(path: &Path, value: &Value) -> Result<OwnedRecipeIngredient, DataPackError> {
    if let Some(array) = value.as_array() {
        let ingredients = array
            .iter()
            .map(|entry| parse_ingredient(path, entry))
            .collect::<Result<Vec<_>, _>>()?;
        if ingredients.is_empty() {
            return Err(invalid_recipe(
                path,
                "ingredient alternatives cannot be empty",
            ));
        }
        let mut ids = Vec::new();
        for ingredient in ingredients {
            match ingredient {
                OwnedRecipeIngredient::Simple(id) => ids.push(id),
                OwnedRecipeIngredient::OneOf(mut alternatives) => ids.append(&mut alternatives),
                OwnedRecipeIngredient::Tagged(_) => {
                    return Err(invalid_recipe(
                        path,
                        "tag alternatives are not representable in OneOf",
                    ));
                }
            }
        }
        return Ok(OwnedRecipeIngredient::OneOf(ids));
    }
    let object = value
        .as_object()
        .ok_or_else(|| invalid_recipe(path, "ingredient must be an object or array"))?;
    if let Some(item) = object.get("item").and_then(Value::as_str) {
        let id = canonicalize_recipe_id(item);
        validate_item(path, &id)?;
        return Ok(OwnedRecipeIngredient::Simple(id));
    }
    if let Some(tag) = object.get("tag").and_then(Value::as_str) {
        return Ok(OwnedRecipeIngredient::Tagged(canonicalize_recipe_id(tag)));
    }
    Err(invalid_recipe(path, "ingredient requires item or tag"))
}

fn parse_result(path: &Path, value: Option<&Value>) -> Result<OwnedRecipeResult, DataPackError> {
    let object = value
        .and_then(Value::as_object)
        .ok_or_else(|| invalid_recipe(path, "result must be an object"))?;
    let item = object
        .get("id")
        .or_else(|| object.get("item"))
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_recipe(path, "result requires id"))?;
    let item_id = canonicalize_recipe_id(item);
    validate_item(path, &item_id)?;
    let count = object.get("count").and_then(Value::as_u64).unwrap_or(1);
    if !(1..=u8::MAX as u64).contains(&count) {
        return Err(invalid_recipe(path, "result count must be in 1..=255"));
    }
    Ok(OwnedRecipeResult {
        item_id,
        count: count as u8,
        components: object
            .get("components")
            .map(json_object_to_nbt)
            .transpose()
            .map_err(|message| {
                invalid_recipe(path, format!("invalid result components: {message}"))
            })?,
    })
}

/// Converts the JSON component payload used by datapacks into the NBT shape
/// consumed by `ItemStack::read_item_stack`.  Datapack components are NBT-like
/// rather than arbitrary JSON, so reject nulls and non-integral numbers instead
/// of silently changing their wire meaning.
fn json_object_to_nbt(value: &Value) -> Result<NbtCompound, String> {
    let Value::Object(object) = value else {
        return Err("components must be an object".to_owned());
    };
    let mut compound = NbtCompound::new();
    for (key, value) in object {
        compound.put(key, json_value_to_nbt(value)?);
    }
    Ok(compound)
}

fn json_value_to_nbt(value: &Value) -> Result<NbtTag, String> {
    match value {
        Value::Null => Err("null is not a valid NBT component value".to_owned()),
        Value::Bool(value) => Ok(NbtTag::Byte(i8::from(*value))),
        Value::String(value) => Ok(NbtTag::String(value.clone().into_boxed_str())),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                if let Ok(value) = i32::try_from(value) {
                    Ok(NbtTag::Int(value))
                } else {
                    Ok(NbtTag::Long(value))
                }
            } else if let Some(value) = value.as_f64() {
                Ok(NbtTag::Double(value))
            } else {
                Err("number is outside NBT numeric range".to_owned())
            }
        }
        Value::Array(values) => {
            let values = values
                .iter()
                .map(json_value_to_nbt)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(NbtTag::List(values))
        }
        Value::Object(object) => {
            let mut compound = NbtCompound::new();
            for (key, value) in object {
                compound.put(key, json_value_to_nbt(value)?);
            }
            Ok(NbtTag::Compound(compound))
        }
    }
}

fn validate_item(path: &Path, id: &str) -> Result<(), DataPackError> {
    let key = id.strip_prefix("minecraft:").unwrap_or(id);
    if Item::from_registry_key(key).is_none() {
        return Err(invalid_recipe(path, format!("unknown item {id}")));
    }
    Ok(())
}

fn parse_category(value: &Value) -> RecipeCategoryTypes {
    match value.get("category").and_then(Value::as_str) {
        Some("building") => RecipeCategoryTypes::Building,
        Some("equipment") => RecipeCategoryTypes::Equipment,
        Some("redstone") => RecipeCategoryTypes::Restone,
        Some("food") => RecipeCategoryTypes::Food,
        Some("blocks") => RecipeCategoryTypes::Blocks,
        _ => RecipeCategoryTypes::Misc,
    }
}

fn recipe_id_from_path(path: &Path) -> Result<String, DataPackError> {
    let components = path
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid_recipe(path, "recipe path is not valid UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let recipes_index = components
        .iter()
        .rposition(|component| component == "recipes")
        .ok_or_else(|| invalid_recipe(path, "recipe is outside a namespace recipes directory"))?;
    let namespace_index = recipes_index
        .checked_sub(1)
        .ok_or_else(|| invalid_recipe(path, "recipe namespace is missing"))?;
    if namespace_index == 0 || components[namespace_index - 1] != "data" {
        return Err(invalid_recipe(path, "recipe is outside a data namespace"));
    }
    let resource_parts = &components[recipes_index + 1..];
    let Some(last) = resource_parts.last() else {
        return Err(invalid_recipe(path, "recipe resource name is missing"));
    };
    if !last.ends_with(".json") {
        return Err(invalid_recipe(path, "recipe resource must end with .json"));
    }
    recipe_id_from_resource_path(
        &format!(
            "data/{}/recipes/{}",
            components[namespace_index],
            resource_parts.join("/")
        ),
        path,
    )
}

fn recipe_id_from_resource_path(resource_path: &str, path: &Path) -> Result<String, DataPackError> {
    let components = resource_path.split('/').collect::<Vec<_>>();
    if components.len() < 4 || components[0] != "data" || components[2] != "recipes" {
        return Err(invalid_recipe(path, "recipe is outside a data namespace"));
    }
    let resource_parts = &components[3..];
    let Some(last) = resource_parts.last() else {
        return Err(invalid_recipe(path, "recipe resource name is missing"));
    };
    if !last.ends_with(".json") {
        return Err(invalid_recipe(path, "recipe resource must end with .json"));
    }
    let mut name = resource_parts.to_vec();
    let last_index = name.len() - 1;
    name[last_index] = last.trim_end_matches(".json");
    Identifier::new(components[1].to_owned(), name.join("/"))
        .map(|identifier| identifier.to_string())
        .map_err(|error| invalid_recipe(path, error.to_string()))
}

fn function_id_from_path(path: &Path) -> Result<String, DataPackError> {
    let components = path
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid_function(path, "function path is not valid UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let function_index = components
        .iter()
        .rposition(|component| component == "function")
        .ok_or_else(|| invalid_function(path, "function is outside a namespace directory"))?;
    let namespace_index = function_index
        .checked_sub(1)
        .ok_or_else(|| invalid_function(path, "function namespace is missing"))?;
    if namespace_index == 0 || components[namespace_index - 1] != "data" {
        return Err(invalid_function(
            path,
            "function is outside a data namespace",
        ));
    }
    let resource_parts = &components[function_index + 1..];
    let Some(last) = resource_parts.last() else {
        return Err(invalid_function(path, "function resource name is missing"));
    };
    if !last.ends_with(".mcfunction") {
        return Err(invalid_function(
            path,
            "function resource must end with .mcfunction",
        ));
    }
    let mut name = resource_parts.to_vec();
    let last_index = name.len() - 1;
    name[last_index] = last.trim_end_matches(".mcfunction").to_owned();
    Identifier::new(components[namespace_index].to_owned(), name.join("/"))
        .map(|identifier| identifier.to_string())
        .map_err(|error| invalid_function(path, error.to_string()))
}

fn function_id_from_resource_path(
    resource_path: &str,
    path: &Path,
) -> Result<String, DataPackError> {
    let components = resource_path.split('/').collect::<Vec<_>>();
    if components.len() < 4 || components[0] != "data" || components[2] != "function" {
        return Err(invalid_function(
            path,
            "function is outside a data namespace function directory",
        ));
    }
    let resource_parts = &components[3..];
    let Some(last) = resource_parts.last() else {
        return Err(invalid_function(path, "function resource name is missing"));
    };
    if !last.ends_with(".mcfunction") {
        return Err(invalid_function(
            path,
            "function resource must end with .mcfunction",
        ));
    }
    let mut name = resource_parts.to_vec();
    let last_index = name.len() - 1;
    name[last_index] = last.trim_end_matches(".mcfunction");
    Identifier::new(components[1].to_owned(), name.join("/"))
        .map(|identifier| identifier.to_string())
        .map_err(|error| invalid_function(path, error.to_string()))
}

fn invalid_function(path: &Path, message: impl Into<String>) -> DataPackError {
    DataPackError::InvalidFunction {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

fn invalid_recipe(path: &Path, message: impl Into<String>) -> DataPackError {
    DataPackError::InvalidRecipe {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{DataPackLoader, TagSnapshot};
    use crate::server::recipe::recipe_id;
    use pumpkin_nbt::compound::NbtCompound;
    use pumpkin_protocol::codec::recipe::{DynamicRecipe, OwnedCraftingRecipe};
    use std::fs;
    use std::io::Write;

    fn structure_bytes(marker: &str) -> Vec<u8> {
        let mut compound = NbtCompound::new();
        compound.put_string("Marker", marker.to_owned());
        pumpkin_nbt::nbt_compress::write_gzip_compound_tag_to_bytes(compound)
            .expect("write structure NBT")
    }

    #[test]
    fn loads_raw_resources_and_overrides_them_transactionally() {
        let root = tempfile::tempdir().expect("tempdir");
        let datapacks = root.path().join("datapacks");
        let first = datapacks.join("first");
        let second = datapacks.join("second");
        for pack in [&first, &second] {
            fs::create_dir_all(pack.join("data/example/loot_tables")).expect("loot dirs");
            fs::create_dir_all(pack.join("data/example/predicates")).expect("predicate dirs");
            fs::create_dir_all(pack.join("data/example/advancements")).expect("advancement dirs");
            fs::create_dir_all(pack.join("data/example/structures")).expect("structure dirs");
            fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"pack_format":71,"description":"test"}}"#,
            )
            .expect("metadata");
        }
        fs::write(
            first.join("data/example/loot_tables/reward.json"),
            r#"{"type":"minecraft:chest","pools":[]}"#,
        )
        .expect("loot table");
        fs::write(
            second.join("data/example/loot_tables/reward.json"),
            r#"{"type":"minecraft:entity","pools":[]}"#,
        )
        .expect("override loot table");
        fs::write(
            first.join("data/example/predicates/day.json"),
            r#"{"condition":"minecraft:location_check"}"#,
        )
        .expect("predicate");
        fs::write(
            second.join("data/example/advancements/root.json"),
            r#"{"criteria":{}}"#,
        )
        .expect("advancement");
        fs::write(
            first.join("data/example/structures/house.nbt"),
            structure_bytes("first"),
        )
        .expect("structure");
        fs::write(
            second.join("data/example/structures/house.nbt"),
            structure_bytes("second"),
        )
        .expect("override structure");

        let snapshot = DataPackLoader::load(
            root.path(),
            &["file/first".to_owned(), "file/second".to_owned()],
        )
        .expect("load raw resources");
        assert_eq!(snapshot.resources.loot_tables.len(), 1);
        assert_eq!(
            snapshot.resources.loot_tables["example:reward"]["type"],
            "minecraft:entity"
        );
        assert!(snapshot.resources.predicates.contains_key("example:day"));
        assert!(snapshot.resources.advancements.contains_key("example:root"));
        assert!(snapshot.resources.structures.contains_key("example:house"));
        assert_ne!(
            snapshot.resources.structures["example:house"],
            structure_bytes("first")
        );
    }

    #[test]
    fn malformed_raw_resource_rejects_the_entire_candidate() {
        let root = tempfile::tempdir().expect("tempdir");
        let pack = root.path().join("datapacks/test");
        fs::create_dir_all(pack.join("data/example/advancements")).expect("advancement dirs");
        fs::create_dir_all(pack.join("data/example/structures")).expect("structure dirs");
        fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"pack_format":71,"description":"test"}}"#,
        )
        .expect("metadata");
        fs::write(
            pack.join("data/example/advancements/broken.json"),
            r#"{"criteria":[]}"#,
        )
        .expect("broken advancement");
        let error = DataPackLoader::load(root.path(), &["file/test".to_owned()])
            .expect_err("invalid advancement must abort loading");
        assert!(error.to_string().contains("advancements"));

        fs::write(
            pack.join("data/example/advancements/broken.json"),
            r#"{"criteria":{}}"#,
        )
        .expect("valid advancement");
        fs::write(pack.join("data/example/structures/broken.nbt"), b"not nbt")
            .expect("broken structure");
        let error = DataPackLoader::load(root.path(), &["file/test".to_owned()])
            .expect_err("invalid structure must abort loading");
        assert!(error.to_string().contains("structures"));
    }

    #[test]
    fn missing_datapack_directory_does_not_clear_an_explicit_pack() {
        let root = tempfile::tempdir().expect("tempdir");
        let error = DataPackLoader::load(root.path(), &["vanilla".to_owned()])
            .expect("implicit vanilla pack needs no directory");
        assert!(error.packs.is_empty());

        let error = DataPackLoader::load(root.path(), &["file/missing".to_owned()])
            .expect_err("an enabled file pack must be reported as missing");
        assert!(error.to_string().contains("missing"));
    }

    #[test]
    fn loads_and_overrides_shaped_recipe_in_priority_order() {
        let root = tempfile::tempdir().expect("tempdir");
        let datapacks = root.path().join("datapacks");
        let first = datapacks.join("first");
        let second = datapacks.join("second");
        for pack in [&first, &second] {
            fs::create_dir_all(pack.join("data/example/recipes")).expect("recipe dirs");
            fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"pack_format":71,"description":"test"}}"#,
            )
            .expect("metadata");
        }
        fs::write(
            first.join("data/example/recipes/tool.json"),
            r##"{"type":"minecraft:crafting_shaped","pattern":["#"],"key":{"#":{"item":"minecraft:oak_planks"}},"result":{"id":"minecraft:stick"}}"##,
        )
        .expect("first recipe");
        fs::write(
            second.join("data/example/recipes/tool.json"),
            r##"{"type":"minecraft:crafting_shaped","pattern":["#"],"key":{"#":{"item":"minecraft:stone"}},"result":{"id":"minecraft:stick"}}"##,
        )
        .expect("second recipe");

        let snapshot = DataPackLoader::load(
            root.path(),
            &["file/first".to_owned(), "file/second".to_owned()],
        )
        .expect("load datapacks");
        assert_eq!(snapshot.packs.len(), 2);
        assert_eq!(snapshot.recipes.len(), 1);
        assert!(snapshot.unsupported_recipe_types.is_empty());
    }

    #[test]
    fn malformed_recipe_rejects_candidate_load() {
        let root = tempfile::tempdir().expect("tempdir");
        let pack = root.path().join("datapacks/test");
        fs::create_dir_all(pack.join("data/example/recipes")).expect("recipe dirs");
        fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"pack_format":71,"description":"test"}}"#,
        )
        .expect("metadata");
        fs::write(
            pack.join("data/example/recipes/broken.json"),
            r##"{"type":"minecraft:crafting_shaped","pattern":["#"],"key":{}}"##,
        )
        .expect("broken recipe");
        assert!(DataPackLoader::load(root.path(), &["file/test".to_owned()]).is_err());
    }

    #[test]
    fn loads_recipe_from_zip_datapack_without_extracting_it() {
        let root = tempfile::tempdir().expect("tempdir");
        let datapacks = root.path().join("datapacks");
        fs::create_dir_all(&datapacks).expect("datapacks");
        let file = fs::File::create(datapacks.join("packed.zip")).expect("zip file");
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("pack.mcmeta", options).expect("metadata");
        writer
            .write_all(br#"{"pack":{"pack_format":71,"description":"test"}}"#)
            .expect("metadata bytes");
        writer
            .start_file("data/example/recipes/nested/tool.json", options)
            .expect("recipe");
        writer
            .write_all(br#"{"type":"minecraft:crafting_shapeless","ingredients":[{"item":"minecraft:oak_planks"}],"result":{"id":"minecraft:stick"}}"#)
            .expect("recipe bytes");
        writer
            .start_file("data/example/tags/items/wood.json", options)
            .expect("tag");
        writer
            .write_all(br#"{"values":["minecraft:oak_planks"]}"#)
            .expect("tag bytes");
        writer
            .start_file("data/example/function/start.mcfunction", options)
            .expect("function");
        writer
            .write_all(b"# comment\n  say hello  \n\n")
            .expect("function bytes");
        writer
            .start_file("data/example/loot_tables/zip.json", options)
            .expect("loot table");
        writer
            .write_all(br#"{"pools":[],"type":"minecraft:chest"}"#)
            .expect("loot table bytes");
        writer
            .start_file("data/example/predicates/zip.json", options)
            .expect("predicate");
        writer
            .write_all(br#"{"condition":"minecraft:location_check"}"#)
            .expect("predicate bytes");
        writer
            .start_file("data/example/advancements/zip.json", options)
            .expect("advancement");
        writer
            .write_all(br#"{"criteria":{}}"#)
            .expect("advancement bytes");
        writer
            .start_file("data/example/structures/zip.nbt", options)
            .expect("structure");
        writer
            .write_all(&structure_bytes("zip"))
            .expect("structure bytes");
        writer.finish().expect("finish zip");

        let snapshot = DataPackLoader::load(root.path(), &["file/packed.zip".to_owned()])
            .expect("load zip datapack");
        assert_eq!(snapshot.packs.len(), 1);
        assert_eq!(snapshot.recipes.len(), 1);
        assert_eq!(recipe_id(&snapshot.recipes[0]), "example:nested/tool");
        assert!(
            snapshot
                .tags
                .contains("example:items/wood", "minecraft:oak_planks")
        );
        assert_eq!(
            snapshot.functions.get("example:start"),
            Some(&vec!["say hello".to_owned()])
        );
        assert!(snapshot.resources.loot_tables.contains_key("example:zip"));
        assert!(snapshot.resources.predicates.contains_key("example:zip"));
        assert!(snapshot.resources.advancements.contains_key("example:zip"));
        assert!(snapshot.resources.structures.contains_key("example:zip"));
    }

    #[test]
    fn functions_override_by_pack_priority_and_reject_unsafe_lines() {
        let root = tempfile::tempdir().expect("tempdir");
        let datapacks = root.path().join("datapacks");
        let first = datapacks.join("first");
        let second = datapacks.join("second");
        for pack in [&first, &second] {
            fs::create_dir_all(pack.join("data/example/function")).expect("function dirs");
            fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"pack_format":71,"description":"test"}}"#,
            )
            .expect("metadata");
        }
        fs::write(
            first.join("data/example/function/start.mcfunction"),
            "say first\n",
        )
        .expect("first function");
        fs::write(
            second.join("data/example/function/start.mcfunction"),
            "say second\n",
        )
        .expect("second function");
        let snapshot = DataPackLoader::load(
            root.path(),
            &["file/first".to_owned(), "file/second".to_owned()],
        )
        .expect("load functions");
        assert_eq!(snapshot.functions["example:start"], vec!["say second"]);

        fs::write(
            second.join("data/example/function/bad.mcfunction"),
            "say\0bad\n",
        )
        .expect("bad function");
        assert!(DataPackLoader::load(root.path(), &["file/second".to_owned()]).is_err());
    }

    #[test]
    fn function_tags_preserve_order_and_require_existing_functions() {
        let root = tempfile::tempdir().expect("tempdir");
        let pack = root.path().join("datapacks/test");
        fs::create_dir_all(pack.join("data/example/function")).expect("function dirs");
        fs::create_dir_all(pack.join("data/example/tags/function")).expect("tag dirs");
        fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"pack_format":71,"description":"test"}}"#,
        )
        .expect("metadata");
        for (name, command) in [("first", "say first\n"), ("second", "say second\n")] {
            fs::write(
                pack.join(format!("data/example/function/{name}.mcfunction")),
                command,
            )
            .expect("function");
        }
        fs::write(
            pack.join("data/example/tags/function/tick.json"),
            r#"{"values":["example:second","example:first","example:second"]}"#,
        )
        .expect("function tag");
        let snapshot = DataPackLoader::load(root.path(), &["file/test".to_owned()])
            .expect("load function tag");
        assert_eq!(
            snapshot.tags.ordered_members("example:function/tick"),
            vec!["example:second", "example:first"]
        );

        fs::write(
            pack.join("data/example/tags/function/broken.json"),
            r#"{"values":["example:missing"]}"#,
        )
        .expect("broken function tag");
        assert!(DataPackLoader::load(root.path(), &["file/test".to_owned()]).is_err());
        fs::write(
            pack.join("data/example/tags/function/broken.json"),
            r#"{"values":[{"id":"example:missing","required":false}]}"#,
        )
        .expect("optional function tag");
        let snapshot = DataPackLoader::load(root.path(), &["file/test".to_owned()])
            .expect("optional missing function is allowed");
        assert_eq!(
            snapshot.tags.ordered_members("example:function/broken"),
            vec!["example:missing"]
        );
    }

    #[test]
    fn tags_follow_pack_priority_replace_and_optional_entries() {
        let root = tempfile::tempdir().expect("tempdir");
        let datapacks = root.path().join("datapacks");
        let first = datapacks.join("first");
        let second = datapacks.join("second");
        for pack in [&first, &second] {
            fs::create_dir_all(pack.join("data/example/tags/items")).expect("tag dirs");
            fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"pack_format":71,"description":"test"}}"#,
            )
            .expect("metadata");
        }
        fs::write(
            first.join("data/example/tags/items/planks.json"),
            r#"{"values":["minecraft:oak_planks"]}"#,
        )
        .expect("first tag");
        fs::write(
            second.join("data/example/tags/items/planks.json"),
            r#"{"replace":true,"values":["minecraft:stone",{"id":"minecraft:not_an_item","required":false}]}"#,
        )
        .expect("second tag");

        let snapshot = DataPackLoader::load(
            root.path(),
            &["file/first".to_owned(), "file/second".to_owned()],
        )
        .expect("load tags");
        let members = snapshot.tags.members("example:items/planks");
        assert_eq!(
            members.into_iter().collect::<Vec<_>>(),
            vec!["minecraft:stone"]
        );
    }

    #[test]
    fn tags_reject_required_missing_references_and_cycles_before_publish() {
        let root = tempfile::tempdir().expect("tempdir");
        let pack = root.path().join("datapacks/test");
        fs::create_dir_all(pack.join("data/example/tags/items")).expect("tag dirs");
        fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"pack_format":71,"description":"test"}}"#,
        )
        .expect("metadata");
        fs::write(
            pack.join("data/example/tags/items/broken.json"),
            r##"{"values":["#example:items/missing"]}"##,
        )
        .expect("broken tag");
        assert!(DataPackLoader::load(root.path(), &["file/test".to_owned()]).is_err());

        fs::write(
            pack.join("data/example/tags/items/broken.json"),
            r##"{"values":["#example:items/broken"]}"##,
        )
        .expect("cyclic tag");
        assert!(DataPackLoader::load(root.path(), &["file/test".to_owned()]).is_err());
    }

    #[test]
    fn empty_tag_snapshot_has_no_membership() {
        let snapshot = TagSnapshot::default();
        assert!(!snapshot.contains("minecraft:items/planks", "minecraft:oak_planks"));
    }

    #[test]
    fn preserves_datapack_result_components_as_nbt() {
        let root = tempfile::tempdir().expect("tempdir");
        let pack = root.path().join("datapacks/components");
        fs::create_dir_all(pack.join("data/example/recipes")).expect("recipe dirs");
        fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"pack_format":71,"description":"test"}}"#,
        )
        .expect("metadata");
        fs::write(
            pack.join("data/example/recipes/component.json"),
            r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"item":"minecraft:oak_planks"}],"result":{"id":"minecraft:stick","components":{"minecraft:custom_data":{"source":"test","count":2}}}}"#,
        )
        .expect("recipe");

        let snapshot = DataPackLoader::load(root.path(), &["file/components".to_owned()])
            .expect("load datapack");
        let DynamicRecipe::Crafting(OwnedCraftingRecipe::Shapeless { result, .. }) =
            &snapshot.recipes[0]
        else {
            panic!("expected shapeless recipe");
        };
        let components = result.components.as_ref().expect("components");
        let custom_data = components
            .get_compound("minecraft:custom_data")
            .expect("custom data");
        assert_eq!(custom_data.get_string("source"), Some("test"));
        assert_eq!(custom_data.get_int("count"), Some(2));
    }
}
