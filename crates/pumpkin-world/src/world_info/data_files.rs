use std::{
    fs::{self, File},
    io::BufWriter,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use pumpkin_data::game_rules::{GameRule, GameRuleRegistry, GameRuleValue};
use pumpkin_nbt::{compound::NbtCompound, nbt_compress::read_gzip_compound_tag, tag::NbtTag};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::world_info::{WorldGenSettings, WorldInfoError};

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct DataFileRoot<T> {
    #[serde(rename = "data")]
    pub data: T,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct WeatherData {
    #[serde(rename = "rain_time", default)]
    pub rain_time: i32,
    #[serde(rename = "raining", default)]
    pub raining: bool,
    #[serde(rename = "thundering", default)]
    pub thundering: bool,
    #[serde(rename = "thunder_time", default)]
    pub thunder_time: i32,
    #[serde(rename = "clear_weather_time", default)]
    pub clear_weather_time: i32,
    #[serde(rename = "DataVersion", default)]
    pub data_version: i32,
}

impl Default for WeatherData {
    fn default() -> Self {
        Self {
            rain_time: 0,
            raining: false,
            thundering: false,
            thunder_time: 0,
            clear_weather_time: -1,
            data_version: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct WorldGenSettingsData {
    #[serde(flatten)]
    pub settings: WorldGenSettings,
    #[serde(rename = "DataVersion", default)]
    pub data_version: i32,
    #[serde(rename = "bonus_chest", default)]
    pub bonus_chest: bool,
    #[serde(rename = "generate_structures", default = "default_true")]
    pub generate_structures: bool,
}

const fn default_true() -> bool {
    true
}

impl WorldGenSettingsData {
    #[must_use]
    pub const fn new(settings: WorldGenSettings, data_version: i32) -> Self {
        Self {
            settings,
            data_version,
            bonus_chest: false,
            generate_structures: true,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DimensionClock {
    pub total_ticks: i64,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct WorldClocksData {
    pub clocks: std::collections::HashMap<String, DimensionClock>,
    pub data_version: i32,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct WanderingTraderData {
    #[serde(rename = "spawn_delay", default = "default_wandering_trader_delay")]
    pub spawn_delay: i32,
    #[serde(rename = "spawn_chance", default = "default_wandering_trader_chance")]
    pub spawn_chance: i32,
    #[serde(rename = "DataVersion", default)]
    pub data_version: i32,
}

/// A vanilla `scheduled_events.dat` function callback.
///
/// The server crate owns queue ordering/execution; this small data type keeps
/// the on-disk codec in the world-info crate so unknown callbacks remain lossless.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledFunctionData {
    pub trigger_time: i64,
    pub id: String,
    pub tag: bool,
}

const fn default_wandering_trader_delay() -> i32 {
    24_000
}
const fn default_wandering_trader_chance() -> i32 {
    25
}

impl Default for WanderingTraderData {
    fn default() -> Self {
        Self {
            spawn_delay: default_wandering_trader_delay(),
            spawn_chance: default_wandering_trader_chance(),
            data_version: 0,
        }
    }
}

#[must_use]
pub fn minecraft_data_dir(level_folder: &Path) -> PathBuf {
    level_folder.join("data").join("minecraft")
}

/// Ensures the `<world>/data/minecraft/` directory exists.
pub fn ensure_minecraft_data_dir(level_folder: &Path) -> Result<PathBuf, WorldInfoError> {
    let dir = minecraft_data_dir(level_folder);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Load an existing data-file root before rewriting it.  Vanilla data files
/// are extensible and datapacks routinely add fields which this runtime does
/// not know yet; replacing the root outright would silently delete them.
fn existing_data_root(path: &Path) -> Result<NbtCompound, WorldInfoError> {
    match File::open(path) {
        Ok(file) => read_gzip_compound_tag(file)
            .map_err(|e| WorldInfoError::DeserializationError(e.to_string())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(NbtCompound::new()),
        Err(error) => Err(error.into()),
    }
}

pub fn read_weather(level_folder: &Path) -> WeatherData {
    let path = minecraft_data_dir(level_folder).join("weather.dat");
    if !path.exists() {
        return WeatherData::default();
    }
    match File::open(&path) {
        Ok(f) => match read_gzip_compound_tag(f) {
            Ok(compound) => {
                let data_compound = compound.get_compound("data");
                let c = data_compound.as_ref().map_or(&compound, |v| v);
                WeatherData {
                    clear_weather_time: c.get_int("clear_weather_time").unwrap_or(0),
                    rain_time: c.get_int("rain_time").unwrap_or(0),
                    thunder_time: c.get_int("thunder_time").unwrap_or(0),
                    raining: c.get_bool("raining").unwrap_or(false),
                    thundering: c.get_bool("thundering").unwrap_or(false),
                    data_version: c.get_int("DataVersion").unwrap_or(0),
                }
            }
            Err(e) => {
                warn!("Failed to deserialize weather.dat, using defaults: {e}");
                WeatherData::default()
            }
        },
        Err(e) => {
            warn!("Failed to open weather.dat, using defaults: {e}");
            WeatherData::default()
        }
    }
}

pub fn write_weather(level_folder: &Path, data: &WeatherData) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("weather.dat");
    let mut root = existing_data_root(&path)?;
    let mut data_comp = root.get_compound("data").cloned().unwrap_or_default();
    data_comp.put_int("DataVersion", data.data_version);
    data_comp.put_int("clear_weather_time", data.clear_weather_time);
    data_comp.put_int("rain_time", data.rain_time);
    data_comp.put_int("thunder_time", data.thunder_time);
    data_comp.put_bool("raining", data.raining);
    data_comp.put_bool("thundering", data.thundering);
    root.put_compound("data", data_comp);
    let file = File::create(&path)?;
    pumpkin_nbt::nbt_compress::write_gzip_compound_tag(root, BufWriter::new(file))
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

pub fn read_world_gen_settings(level_folder: &Path) -> Option<WorldGenSettings> {
    let path = minecraft_data_dir(level_folder).join("world_gen_settings.dat");
    if !path.exists() {
        return None;
    }
    match File::open(&path) {
        Ok(f) => match read_gzip_compound_tag(f) {
            Ok(compound) => {
                let seed = compound
                    .get_compound("data")
                    .and_then(|c| c.get_long("seed"));
                if seed.is_none() {
                    warn!("world_gen_settings.dat has no seed");
                }
                seed.map(|seed| WorldGenSettings {
                    seed,
                    dimensions: std::collections::HashMap::new(),
                })
            }
            Err(e) => {
                warn!("Failed to deserialize world_gen_settings.dat: {e}");
                None
            }
        },
        Err(e) => {
            warn!("Failed to open world_gen_settings.dat: {e}");
            None
        }
    }
}

pub fn write_world_gen_settings(
    level_folder: &Path,
    settings: &WorldGenSettings,
    data_version: i32,
) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("world_gen_settings.dat");
    let mut root = existing_data_root(&path)?;
    let mut inner = root.get_compound("data").cloned().unwrap_or_default();
    inner.put_int("DataVersion", data_version);
    inner.put_long("seed", settings.seed);

    root.put_compound("data", inner);
    let file = File::create(&path)?;
    pumpkin_nbt::nbt_compress::write_gzip_compound_tag(root, BufWriter::new(file))
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

#[must_use]
pub fn game_rules_to_nbt(rules: &GameRuleRegistry, data_version: i32) -> NbtCompound {
    let mut inner = NbtCompound::new();
    for rule in GameRule::all() {
        let key = format!("minecraft:{rule}");
        match rules.get(rule) {
            GameRuleValue::Bool(b) => inner.put(&key, NbtTag::Byte(i8::from(*b))),
            GameRuleValue::Int(i) => inner.put(&key, NbtTag::Int(*i as i32)),
        }
    }
    inner.put_int("DataVersion", data_version);

    let mut root = NbtCompound::new();
    root.put_compound("data", inner);
    root
}

pub fn game_rules_from_nbt(root: &NbtCompound) -> GameRuleRegistry {
    let mut registry = GameRuleRegistry::default();

    let Some(inner) = root.get_compound("data") else {
        warn!("game_rules.dat missing 'data' compound, using defaults");
        return registry;
    };

    for rule in GameRule::all() {
        let key = format!("minecraft:{rule}");
        match registry.get_mut(rule) {
            GameRuleValue::Bool(b) => {
                if let Some(v) = inner.get_byte(&key) {
                    *b = v != 0;
                }
            }
            GameRuleValue::Int(i) => {
                if let Some(v) = inner.get_int(&key) {
                    *i = i64::from(v);
                }
            }
        }
    }

    registry
}

pub fn read_game_rules(level_folder: &Path) -> GameRuleRegistry {
    let path = minecraft_data_dir(level_folder).join("game_rules.dat");
    if !path.exists() {
        return GameRuleRegistry::default();
    }

    match File::open(&path) {
        Ok(f) => match read_gzip_compound_tag(f) {
            Ok(compound) => game_rules_from_nbt(&compound),
            Err(e) => {
                warn!("Failed to parse game_rules.dat: {e}");
                GameRuleRegistry::default()
            }
        },
        Err(e) => {
            warn!("Failed to open game_rules.dat: {e}");
            GameRuleRegistry::default()
        }
    }
}

pub fn write_game_rules(
    level_folder: &Path,
    rules: &GameRuleRegistry,
    data_version: i32,
) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("game_rules.dat");

    let mut root = existing_data_root(&path)?;
    let mut inner = root.get_compound("data").cloned().unwrap_or_default();
    for rule in GameRule::all() {
        let key = format!("minecraft:{rule}");
        match rules.get(rule) {
            GameRuleValue::Bool(value) => inner.put(&key, NbtTag::Byte(i8::from(*value))),
            GameRuleValue::Int(value) => inner.put(&key, NbtTag::Int(*value as i32)),
        }
    }
    inner.put_int("DataVersion", data_version);
    root.put_compound("data", inner);
    let file = File::create(&path)?;

    pumpkin_nbt::nbt_compress::write_gzip_compound_tag(root, file)
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

pub fn read_world_clocks(level_folder: &Path) -> WorldClocksData {
    let path = minecraft_data_dir(level_folder).join("world_clocks.dat");
    if !path.exists() {
        return WorldClocksData::default();
    }

    match File::open(&path) {
        Ok(f) => match read_gzip_compound_tag(f) {
            Ok(compound) => world_clocks_from_nbt(&compound),
            Err(e) => {
                warn!("Failed to parse world_clocks.dat: {e}");
                WorldClocksData::default()
            }
        },
        Err(e) => {
            warn!("Failed to open world_clocks.dat: {e}");
            WorldClocksData::default()
        }
    }
}

fn world_clocks_from_nbt(root: &NbtCompound) -> WorldClocksData {
    let mut result = WorldClocksData::default();

    let Some(inner) = root.get_compound("data") else {
        return result;
    };

    result.data_version = inner.get_int("DataVersion").unwrap_or(0);

    for (key, tag) in &inner.child_tags {
        if key.as_ref() == "DataVersion" {
            continue;
        }
        if let NbtTag::Compound(dim_compound) = tag {
            let total_ticks = dim_compound.get_long("total_ticks").unwrap_or(0);
            result
                .clocks
                .insert(key.to_string(), DimensionClock { total_ticks });
        }
    }

    result
}

pub fn write_world_clocks(
    level_folder: &Path,
    clocks: &WorldClocksData,
) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("world_clocks.dat");

    let mut root = existing_data_root(&path)?;
    let mut inner = root.get_compound("data").cloned().unwrap_or_default();
    for (dim_name, clock) in &clocks.clocks {
        let mut dim_compound = NbtCompound::new();
        dim_compound.put_long("total_ticks", clock.total_ticks);
        inner.put_compound(dim_name, dim_compound);
    }
    inner.put_int("DataVersion", clocks.data_version);

    root.put_compound("data", inner);

    let file = File::create(&path)?;

    pumpkin_nbt::nbt_compress::write_gzip_compound_tag(root, file)
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

pub fn read_wandering_trader(level_folder: &Path) -> WanderingTraderData {
    let path = minecraft_data_dir(level_folder).join("wandering_trader.dat");
    if !path.exists() {
        return WanderingTraderData::default();
    }
    match File::open(&path) {
        Ok(f) => match read_gzip_compound_tag(f) {
            Ok(compound) => {
                let data_compound = compound.get_compound("data");
                let c = data_compound.as_ref().map_or(&compound, |v| v);
                WanderingTraderData {
                    spawn_delay: c.get_int("WanderingTraderSpawnDelay").unwrap_or(24_000),
                    spawn_chance: c.get_int("WanderingTraderSpawnChance").unwrap_or(25),
                    data_version: c.get_int("DataVersion").unwrap_or(0),
                }
            }
            Err(e) => {
                warn!("Failed to deserialize wandering_trader.dat, using defaults: {e}");
                WanderingTraderData::default()
            }
        },
        Err(e) => {
            warn!("Failed to open wandering_trader.dat: {e}");
            WanderingTraderData::default()
        }
    }
}

pub fn write_wandering_trader(
    level_folder: &Path,
    data: &WanderingTraderData,
) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("wandering_trader.dat");
    let mut root = existing_data_root(&path)?;
    let mut data_comp = root.get_compound("data").cloned().unwrap_or_default();
    data_comp.put_int("WanderingTraderSpawnDelay", data.spawn_delay);
    data_comp.put_int("WanderingTraderSpawnChance", data.spawn_chance);
    root.put_compound("data", data_comp);
    let file = File::create(&path)?;
    pumpkin_nbt::nbt_compress::write_gzip_compound_tag(root, BufWriter::new(file))
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

/// Rewrites `custom_boss_events.dat` without discarding boss-bar entries or
/// future fields introduced by a newer vanilla server.
///
/// The live boss-bar manager supplies its own state when available; this
/// low-level writer is deliberately lossless for fields that this crate does
/// not understand yet.
pub fn write_custom_boss_events(
    level_folder: &Path,
    data_version: i32,
) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("custom_boss_events.dat");
    let mut root = existing_data_root(&path)?;
    let mut inner = root.get_compound("data").cloned().unwrap_or_default();
    inner.put_int("DataVersion", data_version);
    root.put_compound("data", inner);
    pumpkin_nbt::nbt_compress::write_gzip_compound_tag(root, File::create(&path)?)
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

/// Rewrites `scheduled_events.dat` while retaining unknown event entries and
/// root/data metadata.
///
/// The actual block/fluid tick queues are persisted in chunk NBT; this file is
/// reserved for global scheduled events.
pub fn write_scheduled_events(
    level_folder: &Path,
    data_version: i32,
) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("scheduled_events.dat");
    let mut root = existing_data_root(&path)?;
    let mut inner = root.get_compound("data").cloned().unwrap_or_default();
    if !inner.has("events") {
        inner.put("events", NbtTag::List(vec![]));
    }
    inner.put_int("DataVersion", data_version);
    root.put_compound("data", inner);
    pumpkin_nbt::nbt_compress::write_gzip_compound_tag(root, File::create(&path)?)
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

/// Reads vanilla function/function-tag callbacks from `scheduled_events.dat`.
/// Unknown callback types and malformed entries are intentionally ignored and
/// preserved by the next rewrite.
#[must_use]
pub fn read_scheduled_functions(level_folder: &Path) -> Vec<ScheduledFunctionData> {
    let path = minecraft_data_dir(level_folder).join("scheduled_events.dat");
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let Ok(root) = read_gzip_compound_tag(file) else {
        return Vec::new();
    };
    let data = root.get_compound("data").unwrap_or(&root);
    let Some(events) = data.get_list("events") else {
        return Vec::new();
    };
    events
        .iter()
        .filter_map(|event| {
            let event = event.extract_compound()?;
            let trigger_time = event.get_long("trigger_time")?;
            let callback = event.get_compound("callback")?;
            let callback_type = callback.get_string("type")?;
            let tag = match callback_type {
                "minecraft:function" => false,
                "minecraft:function_tag" => true,
                _ => return None,
            };
            let id = callback.get_string("id")?.to_owned();
            Some(ScheduledFunctionData {
                trigger_time,
                id,
                tag,
            })
        })
        .collect()
}

/// Rewrites only Pumpkin/vanilla function callbacks in `scheduled_events.dat`.
/// Unknown future callback types and root/data fields survive unchanged.
pub fn write_scheduled_functions(
    level_folder: &Path,
    data_version: i32,
    events: &[ScheduledFunctionData],
) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("scheduled_events.dat");
    let mut root = existing_data_root(&path)?;
    let mut inner = root.get_compound("data").cloned().unwrap_or_default();
    let mut preserved = inner
        .get_list("events")
        .map(|values| {
            values
                .iter()
                .filter(|event| {
                    let Some(event) = event.extract_compound() else {
                        return true;
                    };
                    let Some(callback) = event.get_compound("callback") else {
                        return true;
                    };
                    !matches!(
                        callback.get_string("type"),
                        Some("minecraft:function" | "minecraft:function_tag")
                    )
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    preserved.extend(events.iter().map(|event| {
        let mut callback = NbtCompound::new();
        callback.put_string(
            "type",
            if event.tag {
                "minecraft:function_tag".to_owned()
            } else {
                "minecraft:function".to_owned()
            },
        );
        callback.put_string("id", event.id.clone());
        let mut packed = NbtCompound::new();
        packed.put_long("trigger_time", event.trigger_time);
        packed.put_string(
            "id",
            if event.tag {
                format!("#{}", event.id)
            } else {
                event.id.clone()
            },
        );
        packed.put_compound("callback", callback);
        NbtTag::Compound(packed)
    }));
    inner.put_list("events", preserved);
    inner.put_int("DataVersion", data_version);
    root.put_compound("data", inner);
    pumpkin_nbt::nbt_compress::write_gzip_compound_tag(root, File::create(path)?)
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

/// Backwards-compatible aliases for plugins compiled against the original
/// helper names.  They now use the lossless writer above rather than a
/// create-once stub.
#[deprecated(note = "use write_custom_boss_events")]
pub fn write_custom_boss_events_stub(
    level_folder: &Path,
    data_version: i32,
) -> Result<(), WorldInfoError> {
    write_custom_boss_events(level_folder, data_version)
}

#[deprecated(note = "use write_scheduled_events")]
pub fn write_scheduled_events_stub(
    level_folder: &Path,
    data_version: i32,
) -> Result<(), WorldInfoError> {
    write_scheduled_events(level_folder, data_version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_nbt::nbt_compress::{read_gzip_compound_tag, write_gzip_compound_tag};
    use tempfile::tempdir;

    #[test]
    fn weather_rewrite_preserves_unknown_root_and_data_tags() {
        let directory = tempdir().unwrap();
        let path = minecraft_data_dir(directory.path()).join("weather.dat");
        ensure_minecraft_data_dir(directory.path()).unwrap();

        let mut data = NbtCompound::new();
        data.put_string("datapack_weather_mode", "monsoon".to_string());
        let mut root = NbtCompound::new();
        root.put_string("FutureRootTag", "kept".to_string());
        root.put_compound("data", data);
        write_gzip_compound_tag(root, File::create(&path).unwrap()).unwrap();

        write_weather(directory.path(), &WeatherData::default()).unwrap();
        let rewritten = read_gzip_compound_tag(File::open(path).unwrap()).unwrap();
        assert_eq!(rewritten.get_string("FutureRootTag"), Some("kept"));
        assert_eq!(
            rewritten
                .get_compound("data")
                .and_then(|data| data.get_string("datapack_weather_mode")),
            Some("monsoon")
        );
    }

    #[test]
    fn game_rules_rewrite_preserves_unknown_rule_keys() {
        let directory = tempdir().unwrap();
        let path = minecraft_data_dir(directory.path()).join("game_rules.dat");
        ensure_minecraft_data_dir(directory.path()).unwrap();

        let mut data = NbtCompound::new();
        data.put_int("minecraft:future_rule", 7);
        let mut root = NbtCompound::new();
        root.put_compound("data", data);
        write_gzip_compound_tag(root, File::create(&path).unwrap()).unwrap();

        write_game_rules(directory.path(), &GameRuleRegistry::default(), 4903).unwrap();
        let rewritten = read_gzip_compound_tag(File::open(path).unwrap()).unwrap();
        assert_eq!(
            rewritten
                .get_compound("data")
                .and_then(|data| data.get_int("minecraft:future_rule")),
            Some(7)
        );
    }

    #[test]
    fn global_event_files_upgrade_version_without_losing_unknown_data() {
        let directory = tempdir().unwrap();
        let data_dir = ensure_minecraft_data_dir(directory.path()).unwrap();

        let mut boss_data = NbtCompound::new();
        boss_data.put_string("future_boss", "kept".to_string());
        let mut boss_root = NbtCompound::new();
        boss_root.put_string("FutureRoot", "kept".to_string());
        boss_root.put_compound("data", boss_data);
        write_gzip_compound_tag(
            boss_root,
            File::create(data_dir.join("custom_boss_events.dat")).unwrap(),
        )
        .unwrap();

        write_custom_boss_events(directory.path(), 4903).unwrap();
        let boss =
            read_gzip_compound_tag(File::open(data_dir.join("custom_boss_events.dat")).unwrap())
                .unwrap();
        assert_eq!(boss.get_string("FutureRoot"), Some("kept"));
        assert_eq!(
            boss.get_compound("data")
                .and_then(|data| data.get_string("future_boss")),
            Some("kept")
        );
        assert_eq!(
            boss.get_compound("data")
                .and_then(|data| data.get_int("DataVersion")),
            Some(4903)
        );

        write_scheduled_events(directory.path(), 4903).unwrap();
        let scheduled =
            read_gzip_compound_tag(File::open(data_dir.join("scheduled_events.dat")).unwrap())
                .unwrap();
        assert!(
            scheduled
                .get_compound("data")
                .is_some_and(|data| data.get("events").is_some())
        );
    }

    #[test]
    fn scheduled_function_callbacks_round_trip_and_preserve_unknown_events() {
        let directory = tempdir().unwrap();
        ensure_minecraft_data_dir(directory.path()).unwrap();
        let mut unknown_callback = NbtCompound::new();
        unknown_callback.put_string("type", "example:future_callback".to_owned());
        let mut unknown_event = NbtCompound::new();
        unknown_event.put_long("trigger_time", 9);
        unknown_event.put_string("id", "future".to_owned());
        unknown_event.put_compound("callback", unknown_callback);
        let mut root = NbtCompound::new();
        let mut data = NbtCompound::new();
        data.put_list("events", vec![NbtTag::Compound(unknown_event)]);
        root.put_compound("data", data);
        write_gzip_compound_tag(
            root,
            File::create(minecraft_data_dir(directory.path()).join("scheduled_events.dat"))
                .unwrap(),
        )
        .unwrap();

        let events = vec![
            ScheduledFunctionData {
                trigger_time: 42,
                id: "example:one".to_owned(),
                tag: false,
            },
            ScheduledFunctionData {
                trigger_time: 45,
                id: "example:tick".to_owned(),
                tag: true,
            },
        ];
        write_scheduled_functions(directory.path(), 4903, &events).unwrap();
        assert_eq!(read_scheduled_functions(directory.path()), events);
        let saved = read_gzip_compound_tag(
            File::open(minecraft_data_dir(directory.path()).join("scheduled_events.dat")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            saved
                .get_compound("data")
                .unwrap()
                .get_list("events")
                .unwrap()
                .len(),
            3
        );
    }
}
