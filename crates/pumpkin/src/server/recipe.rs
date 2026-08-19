use std::collections::{BTreeMap, HashSet};

use pumpkin_inventory::crafting::recipe_provider::RecipeProvider;
use pumpkin_inventory::slot::BoxFuture;
pub use pumpkin_protocol::codec::recipe::DynamicRecipe;
use tokio::sync::RwLock;

use super::datapack::{DataPackResources, TagSnapshot};

/// Returns the stable recipe-book key used by Pumpkin for a dynamic recipe.
///
/// The current protocol representation does not retain a separate recipe key
/// for dynamic crafting displays, so crafting recipes use their result item
/// identifier (matching the existing `/recipe` command behavior), while
/// cooking recipes retain their vanilla recipe id.
#[must_use]
pub fn recipe_id(recipe: &DynamicRecipe) -> String {
    match recipe {
        DynamicRecipe::Crafting(crafting) => match crafting {
            pumpkin_protocol::codec::recipe::OwnedCraftingRecipe::Shaped { recipe_id, .. }
            | pumpkin_protocol::codec::recipe::OwnedCraftingRecipe::Shapeless {
                recipe_id, ..
            } => canonicalize_recipe_id(recipe_id),
        },
        DynamicRecipe::Cooking(cooking) => match cooking {
            pumpkin_protocol::codec::recipe::OwnedCookingRecipeType::Smelting(r)
            | pumpkin_protocol::codec::recipe::OwnedCookingRecipeType::Blasting(r)
            | pumpkin_protocol::codec::recipe::OwnedCookingRecipeType::Smoking(r)
            | pumpkin_protocol::codec::recipe::OwnedCookingRecipeType::CampfireCooking(r) => {
                canonicalize_recipe_id(&r.recipe_id)
            }
        },
    }
}

#[must_use]
pub fn canonicalize_recipe_id(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// Vanilla's server-side limited-crafting gate, shared by result-slot
/// commits and serverbound place-recipe requests.  Keeping canonicalization in
/// this helper prevents an unlocked `stick` and a requested `minecraft:stick`
/// from becoming two different keys.
#[must_use]
pub fn is_recipe_allowed(
    limited_crafting: bool,
    known_recipes: &HashSet<String>,
    recipe_id: &str,
) -> bool {
    !limited_crafting || known_recipes.contains(&canonicalize_recipe_id(recipe_id))
}

pub struct RecipeManager {
    runtime: RwLock<RecipeRuntimeState>,
}

#[derive(Clone, Debug, Default)]
struct RecipeRuntimeState {
    dynamic_recipes: Vec<DynamicRecipe>,
    tags: TagSnapshot,
    functions: BTreeMap<String, Vec<String>>,
    resources: DataPackResources,
    generation: u64,
}

impl Default for RecipeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RecipeManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            runtime: RwLock::new(RecipeRuntimeState::default()),
        }
    }

    pub async fn add_recipe(&self, recipe: DynamicRecipe) {
        let mut runtime = self.runtime.write().await;
        let id = recipe_id(&recipe);
        if let Some(existing) = runtime
            .dynamic_recipes
            .iter_mut()
            .find(|existing| recipe_id(existing) == id)
        {
            *existing = recipe;
        } else {
            runtime.dynamic_recipes.push(recipe);
        }
        runtime.generation = runtime.generation.wrapping_add(1);
    }

    /// Publishes a fully validated datapack/plugin snapshot in one write-lock
    /// operation. Callers must build and validate the vector before invoking
    /// this method so reload failures leave the previous snapshot untouched.
    pub async fn replace_dynamic_recipes(&self, recipes: Vec<DynamicRecipe>) {
        let mut runtime = self.runtime.write().await;
        runtime.dynamic_recipes = recipes;
        runtime.generation = runtime.generation.wrapping_add(1);
    }

    /// Publishes the complete datapack runtime (recipes and tags) under one
    /// lock.  Readers that obtain either view cannot observe a half-reloaded
    /// datapack, which is the same prepare/apply barrier used by vanilla's
    /// reload listeners.
    pub async fn replace_datapack_snapshot(
        &self,
        recipes: Vec<DynamicRecipe>,
        tags: TagSnapshot,
        functions: BTreeMap<String, Vec<String>>,
        resources: DataPackResources,
    ) {
        let mut runtime = self.runtime.write().await;
        runtime.dynamic_recipes = recipes;
        runtime.tags = tags;
        runtime.functions = functions;
        runtime.resources = resources;
        runtime.generation = runtime.generation.wrapping_add(1);
    }

    /// Returns the immutable non-recipe resource snapshot published by the
    /// last successful datapack reload. Consumers may hold this clone while a
    /// later reload prepares a replacement without observing partial state.
    pub async fn datapack_resources(&self) -> DataPackResources {
        self.runtime.read().await.resources.clone()
    }

    pub async fn get_dynamic_recipes_internal(&self) -> Vec<DynamicRecipe> {
        self.runtime.read().await.dynamic_recipes.clone()
    }

    pub async fn tag_snapshot(&self) -> TagSnapshot {
        self.runtime.read().await.tags.clone()
    }

    pub async fn function(&self, id: &str) -> Option<Vec<String>> {
        self.runtime.read().await.functions.get(id).cloned()
    }

    pub async fn function_ids(&self) -> Vec<String> {
        self.runtime
            .read()
            .await
            .functions
            .keys()
            .cloned()
            .collect()
    }

    #[must_use]
    pub async fn datapack_generation(&self) -> u64 {
        self.runtime.read().await.generation
    }

    /// Returns the canonical keys that can currently be referenced by a
    /// recipe-book entry.  The snapshot includes generated vanilla recipes
    /// and the atomically published datapack/plugin recipes.  Callers use the
    /// result at persistence and reload boundaries so an old player file
    /// cannot unlock a recipe that no longer exists.
    pub async fn valid_recipe_ids(&self) -> HashSet<String> {
        let mut ids = Self::built_in_recipe_ids();
        for recipe in self.runtime.read().await.dynamic_recipes.iter() {
            ids.insert(recipe_id(recipe));
        }
        ids
    }

    /// Returns the generated vanilla recipe keys independently of the active
    /// datapack snapshot. Commands need this split when they add or remove a
    /// single recipe: the Java recipe-book packet carries generated and
    /// datapack displays in one stream, while the server stores both in one
    /// per-player set.
    #[must_use]
    pub fn built_in_recipe_ids() -> HashSet<String> {
        use pumpkin_data::recipes::{
            CookingRecipeType, CraftingRecipeTypes, RECIPES_COOKING, RECIPES_CRAFTING,
        };

        let mut ids = HashSet::new();
        for recipe in RECIPES_CRAFTING {
            let id = match recipe {
                CraftingRecipeTypes::CraftingShaped { recipe_id, .. }
                | CraftingRecipeTypes::CraftingShapeless { recipe_id, .. }
                | CraftingRecipeTypes::CraftingTransmute { recipe_id, .. }
                | CraftingRecipeTypes::CraftingDecoratedPot { recipe_id, .. } => *recipe_id,
                CraftingRecipeTypes::CraftingSpecial => continue,
            };
            ids.insert(canonicalize_recipe_id(id));
        }
        for recipe in RECIPES_COOKING {
            let id = match recipe {
                CookingRecipeType::Blasting(recipe)
                | CookingRecipeType::Smelting(recipe)
                | CookingRecipeType::Smoking(recipe)
                | CookingRecipeType::CampfireCooking(recipe) => recipe.recipe_id,
            };
            ids.insert(canonicalize_recipe_id(id));
        }

        ids
    }

    /// Resolves the numeric display ID used by the Java recipe-book packets to
    /// a dynamic recipe key. Built-in displays occupy the prefix; dynamic
    /// recipes are emitted after that prefix by `CRecipeBookAdd`.
    pub async fn recipe_id_from_display_id(&self, display_id: i32) -> Option<String> {
        use pumpkin_data::recipes::{CraftingRecipeTypes, RECIPES_COOKING, RECIPES_CRAFTING};

        if display_id < 0 {
            return None;
        }
        let display_id = usize::try_from(display_id).ok()?;
        let mut crafting = RECIPES_CRAFTING.iter().filter(|recipe| {
            !matches!(
                recipe,
                CraftingRecipeTypes::CraftingSpecial
                    | CraftingRecipeTypes::CraftingDecoratedPot { .. }
            )
        });
        let crafting_count = crafting.clone().count();

        if display_id < crafting_count {
            return crafting.nth(display_id).and_then(|recipe| match recipe {
                CraftingRecipeTypes::CraftingShaped { recipe_id, .. }
                | CraftingRecipeTypes::CraftingShapeless { recipe_id, .. }
                | CraftingRecipeTypes::CraftingTransmute { recipe_id, .. } => {
                    Some((*recipe_id).to_owned())
                }
                CraftingRecipeTypes::CraftingSpecial
                | CraftingRecipeTypes::CraftingDecoratedPot { .. } => None,
            });
        }

        let cooking_index = display_id.checked_sub(crafting_count)?;
        if let Some(recipe) = RECIPES_COOKING.get(cooking_index) {
            return Some(match recipe {
                pumpkin_data::recipes::CookingRecipeType::Blasting(recipe)
                | pumpkin_data::recipes::CookingRecipeType::Smelting(recipe)
                | pumpkin_data::recipes::CookingRecipeType::Smoking(recipe)
                | pumpkin_data::recipes::CookingRecipeType::CampfireCooking(recipe) => {
                    recipe.recipe_id.to_owned()
                }
            });
        }

        let dynamic_index = cooking_index.checked_sub(RECIPES_COOKING.len())?;
        self.runtime
            .read()
            .await
            .dynamic_recipes
            .get(dynamic_index)
            .map(recipe_id)
    }
}

impl RecipeProvider for RecipeManager {
    fn get_dynamic_recipes(&self) -> BoxFuture<'_, Vec<DynamicRecipe>> {
        Box::pin(async move { self.runtime.read().await.dynamic_recipes.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DataPackResources, RecipeManager, canonicalize_recipe_id, is_recipe_allowed, recipe_id,
    };
    use pumpkin_data::recipes::RecipeCategoryTypes;
    use pumpkin_protocol::codec::recipe::{
        DynamicRecipe, OwnedCraftingRecipe, OwnedRecipeIngredient, OwnedRecipeResult,
    };
    use serde_json::json;

    #[test]
    fn recipe_ids_are_namespaced_for_vanilla_nbt() {
        assert_eq!(canonicalize_recipe_id("stick"), "minecraft:stick");
        assert_eq!(canonicalize_recipe_id("minecraft:stick"), "minecraft:stick");
        assert_eq!(canonicalize_recipe_id("example:stick"), "example:stick");
    }

    #[test]
    fn limited_crafting_requires_the_canonical_known_recipe_key() {
        let known = ["minecraft:stick".to_owned()].into_iter().collect();
        assert!(is_recipe_allowed(true, &known, "stick"));
        assert!(is_recipe_allowed(
            false,
            &std::collections::HashSet::new(),
            "unknown:recipe"
        ));
        assert!(!is_recipe_allowed(true, &known, "minecraft:torch"));
    }

    #[tokio::test]
    async fn crafting_recipe_keys_are_not_derived_from_output_items() {
        let manager = RecipeManager::new();
        let make_recipe = |id: &str| {
            DynamicRecipe::Crafting(OwnedCraftingRecipe::Shapeless {
                recipe_id: id.to_owned(),
                category: RecipeCategoryTypes::Misc,
                group: None,
                ingredients: vec![OwnedRecipeIngredient::Simple(
                    "minecraft:oak_planks".to_owned(),
                )],
                result: OwnedRecipeResult {
                    item_id: "minecraft:stick".to_owned(),
                    count: 4,
                    components: None,
                },
            })
        };

        manager.add_recipe(make_recipe("example:first")).await;
        manager.add_recipe(make_recipe("example:second")).await;

        let recipes = manager.get_dynamic_recipes_internal().await;
        assert_eq!(recipes.len(), 2);
        assert_eq!(recipe_id(&recipes[0]), "example:first");
        assert_eq!(recipe_id(&recipes[1]), "example:second");
    }

    #[tokio::test]
    async fn built_in_display_ids_resolve_to_stable_recipe_keys() {
        use pumpkin_data::recipes::{CraftingRecipeTypes, RECIPES_CRAFTING};

        let manager = RecipeManager::new();
        let first = RECIPES_CRAFTING
            .iter()
            .find_map(|recipe| match recipe {
                CraftingRecipeTypes::CraftingShaped { recipe_id, .. }
                | CraftingRecipeTypes::CraftingShapeless { recipe_id, .. }
                | CraftingRecipeTypes::CraftingTransmute { recipe_id, .. } => {
                    Some((*recipe_id).to_owned())
                }
                _ => None,
            })
            .expect("generated recipes must contain a displayable recipe");
        assert_eq!(manager.recipe_id_from_display_id(0).await, Some(first));
    }

    #[tokio::test]
    async fn valid_recipe_snapshot_contains_vanilla_and_dynamic_keys() {
        let manager = RecipeManager::new();
        assert!(manager.valid_recipe_ids().await.contains("minecraft:stick"));
        manager
            .add_recipe(DynamicRecipe::Crafting(OwnedCraftingRecipe::Shapeless {
                recipe_id: "example:custom".to_owned(),
                category: RecipeCategoryTypes::Misc,
                group: None,
                ingredients: vec![OwnedRecipeIngredient::Simple(
                    "minecraft:oak_planks".to_owned(),
                )],
                result: OwnedRecipeResult {
                    item_id: "minecraft:stick".to_owned(),
                    count: 1,
                    components: None,
                },
            }))
            .await;
        assert!(manager.valid_recipe_ids().await.contains("example:custom"));
    }

    #[tokio::test]
    async fn datapack_resources_publish_atomically_with_the_recipe_generation() {
        let manager = RecipeManager::new();
        let mut resources = DataPackResources::default();
        resources
            .loot_tables
            .insert("example:test".to_owned(), json!({"pools": []}));
        resources.predicates.insert(
            "example:test".to_owned(),
            json!({"condition": "minecraft:location_check"}),
        );
        let tags = super::TagSnapshot::default();
        manager
            .replace_datapack_snapshot(Vec::new(), tags, Default::default(), resources.clone())
            .await;

        assert_eq!(manager.datapack_generation().await, 1);
        assert_eq!(manager.datapack_resources().await, resources);

        // A recipe-only update must not publish a half-cleared resource
        // registry: plugin/runtime callers share the same immutable snapshot.
        manager.replace_dynamic_recipes(Vec::new()).await;
        assert_eq!(manager.datapack_resources().await, resources);
        assert_eq!(manager.datapack_generation().await, 2);
    }
}
