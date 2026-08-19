//! Crafting screen handler implementation.
//!
//! This module provides screen handlers for crafting mechanics:
//! - [`CraftingScreenHandler`] - Trait for crafting screen handlers
//! - [`CraftingTableScreenHandler`] - The 3x3 crafting table UI
//! - [`ResultSlot`] - The special result slot that shows crafted items
//!
//! # Recipe Matching
//!
//! Crafting recipes are matched against the items in the crafting grid.
//! The system supports:
//! - Shaped recipes (specific patterns)
//! - Shapeless recipes (any arrangement)
//! - Transmute recipes (upgrading items)
//! - Special recipes (like decorated pots)

use std::any::Any;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use super::recipe_provider::{GenericRecipe, RecipeProvider};
use super::recipes::{RecipeFinderScreenHandler, RecipeInputInventory};
use crate::crafting::crafting_inventory::CraftingInventory;
use crate::player::player_inventory::PlayerInventory;
use crate::screen_handler::{
    InventoryPlayer, ItemStackFuture, ScreenHandler, ScreenHandlerBehaviour, ScreenHandlerFuture,
    ScreenHandlerListener, offer_or_drop_stack,
};
use crate::slot::{BoxFuture, NormalSlot, Slot};

use pumpkin_data::data_component_impl::UseRemainderImpl;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::recipes::{CraftingRecipeTypes, RECIPES_CRAFTING};
use pumpkin_data::screen::WindowType;
use pumpkin_data::statistic::StatisticCategory;
use pumpkin_data::tag;
use pumpkin_data::tag::Taggable;
use pumpkin_protocol::codec::recipe::{DynamicRecipe, OwnedCraftingRecipe};
use pumpkin_world::inventory::Inventory;
use tokio::sync::Mutex;

/// The result slot in a crafting screen.
pub struct ResultSlot {
    /// The crafting inventory (grid) that provides recipe input.
    pub inventory: Arc<dyn RecipeInputInventory>,
    /// Protocol ID for this slot (assigned by screen handler).
    pub id: AtomicU8,
    /// The cached result item stack.
    pub result: Arc<Mutex<ItemStack>>,
    /// Provider for dynamic recipes.
    pub recipe_provider: Option<Arc<dyn RecipeProvider>>,
}

pub struct RecipeResult {
    pub recipe_id: String,
    pub item_id: String,
    pub count: u8,
    pub components: Option<pumpkin_nbt::compound::NbtCompound>,
}

/// Recipe-book and limited-crafting checks use resource locations.  Generated
/// vanilla data is normally already namespaced, while plugin/datapack payloads
/// are allowed to omit `minecraft:`; normalize both paths at the matcher
/// boundary so a recipe can never be learned under a different spelling than
/// the one used for crafting.
fn canonicalize_recipe_id(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// Finds the first vanilla or dynamic crafting recipe that matches an input
/// grid.  This is intentionally shared by regular crafting screens and the
/// Crafter block entity so both paths use exactly the same shaped/shapeless
/// matching rules (including mirrored shaped recipes).
pub async fn find_crafting_result(
    inventory: &dyn RecipeInputInventory,
    provider: Option<&dyn RecipeProvider>,
) -> Option<RecipeResult> {
    let mut count = 0usize;
    let width = inventory.get_width();
    let mut top_x = width;
    let mut top_y = inventory.get_height();
    let mut bottom_x = 0usize;
    let mut bottom_y = 0usize;

    for index in 0..inventory.size() {
        if !inventory.is_slot_enabled(index) {
            continue;
        }
        let slot = inventory.get_stack(index).await;
        if !slot.is_empty() {
            let x = index % width;
            let y = index / width;
            top_x = top_x.min(x);
            top_y = top_y.min(y);
            bottom_x = bottom_x.max(x);
            bottom_y = bottom_y.max(y);
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }

    let input_width = bottom_x + 1 - top_x;
    let input_height = bottom_y + 1 - top_y;
    for recipe in RECIPES_CRAFTING {
        if let Some(result) = recipe_matches(
            GenericRecipe::Vanilla(recipe),
            input_height,
            input_width,
            top_x,
            top_y,
            count,
            inventory,
        )
        .await
        {
            return Some(result);
        }
    }

    if let Some(provider) = provider {
        for recipe in provider.get_dynamic_recipes().await {
            if let DynamicRecipe::Crafting(crafting) = &recipe
                && let Some(result) = recipe_matches(
                    GenericRecipe::Dynamic(crafting),
                    input_height,
                    input_width,
                    top_x,
                    top_y,
                    count,
                    inventory,
                )
                .await
            {
                return Some(result);
            }
        }
    }
    None
}

/// Checks if a recipe pattern is symmetrical horizontally.
fn is_symmetrical_horizontally(pattern: &[&str]) -> bool {
    let width = pattern.first().map_or(0, |s| s.len());
    for row in pattern {
        if row.len() != width {
            return false;
        }
        for j in 0..width / 2 {
            if row.chars().nth(j) != row.chars().nth(width - j - 1) {
                return false;
            }
        }
    }
    true
}

/// Checks if a crafting recipe matches the current inventory state.
#[expect(clippy::too_many_lines)]
async fn recipe_matches(
    recipe: GenericRecipe<'_>,
    input_height: usize,
    input_width: usize,
    top_x: usize,
    top_y: usize,
    count: usize,
    inventory: &dyn RecipeInputInventory,
) -> Option<RecipeResult> {
    match recipe {
        GenericRecipe::Vanilla(CraftingRecipeTypes::CraftingShaped {
            recipe_id,
            key,
            pattern,
            result,
            ..
        }) => {
            #[allow(clippy::redundant_closure_for_method_calls)]
            if pattern.len() != input_height
                || pattern.first().map_or(0, |f| f.len()) != input_width
            {
                return None;
            }

            if count
                != pattern
                    .iter()
                    .map(|l| l.chars().filter(|c| *c != ' ').count())
                    .sum::<usize>()
            {
                return None;
            }

            let x_offset = top_x;
            let y_offset = top_y;

            let mut matched = true;
            'outer: for (y, row_str) in pattern.iter().enumerate() {
                for (x, current_key) in row_str.chars().enumerate() {
                    let slot = inventory
                        .get_stack((y + y_offset) * inventory.get_width() + (x + x_offset))
                        .await;
                    let slot_index = (y + y_offset) * inventory.get_width() + (x + x_offset);
                    let slot = if inventory.is_slot_enabled(slot_index) {
                        slot
                    } else {
                        ItemStack::EMPTY.clone()
                    };
                    if current_key == ' ' {
                        if !slot.is_empty() {
                            matched = false;
                            break 'outer;
                        }
                        continue;
                    }

                    let Some(ingredient) = key
                        .iter()
                        .find_map(|(k, v)| (*k == current_key).then_some(v))
                    else {
                        matched = false;
                        break 'outer;
                    };

                    if !ingredient.match_item(slot.item) {
                        matched = false;
                        break 'outer;
                    }
                }
            }

            if !matched && !is_symmetrical_horizontally(pattern) {
                matched = true;
                'outer: for y in 0..pattern.len() {
                    for x in 0..pattern[y].len() {
                        let Some(current_key) = pattern[y].chars().nth(x) else {
                            matched = false;
                            break 'outer;
                        };
                        let slot = inventory
                            .get_stack(
                                (y + y_offset) * inventory.get_width()
                                    + (x_offset + input_width - 1 - x),
                            )
                            .await;
                        let slot_index = (y + y_offset) * inventory.get_width()
                            + (x_offset + input_width - 1 - x);
                        let slot = if inventory.is_slot_enabled(slot_index) {
                            slot
                        } else {
                            ItemStack::EMPTY.clone()
                        };
                        if current_key == ' ' {
                            if !slot.is_empty() {
                                matched = false;
                                break 'outer;
                            }
                            continue;
                        }
                        let Some(ingredient) = key
                            .iter()
                            .find_map(|(k, v)| (*k == current_key).then_some(v))
                        else {
                            matched = false;
                            break 'outer;
                        };
                        if !ingredient.match_item(slot.item) {
                            matched = false;
                            break 'outer;
                        }
                    }
                }
            }

            matched.then_some(RecipeResult {
                recipe_id: canonicalize_recipe_id(recipe_id),
                item_id: result.id.to_string(),
                count: result.count,
                components: None,
            })
        }
        GenericRecipe::Vanilla(CraftingRecipeTypes::CraftingShapeless {
            recipe_id,
            ingredients,
            result,
            ..
        }) => {
            if count != ingredients.len() {
                return None;
            }
            let mut ingredient_used = vec![false; ingredients.len()];
            'next_slot: for i in 0..inventory.size() {
                if !inventory.is_slot_enabled(i) {
                    continue 'next_slot;
                }
                let slot = inventory.get_stack(i).await;
                if slot.is_empty() {
                    continue 'next_slot;
                }
                for i in 0..ingredients.len() {
                    if !ingredient_used[i] && ingredients[i].match_item(slot.item) {
                        ingredient_used[i] = true;
                        continue 'next_slot;
                    }
                }
                return None;
            }
            Some(RecipeResult {
                recipe_id: canonicalize_recipe_id(recipe_id),
                item_id: result.id.to_string(),
                count: result.count,
                components: None,
            })
        }
        GenericRecipe::Vanilla(CraftingRecipeTypes::CraftingTransmute {
            recipe_id,
            input,
            material,
            material_count_min,
            material_count_max,
            add_material_count_to_result,
            result,
            ..
        }) => {
            if count < usize::from(*material_count_min) + 1
                || count > usize::from(*material_count_max) + 1
            {
                return None;
            }
            let mut input_stack = None;
            let mut material_count = 0usize;
            'item_stack: for i in 0..inventory.size() {
                if !inventory.is_slot_enabled(i) {
                    continue 'item_stack;
                }
                let slot = inventory.get_stack(i).await;
                if slot.is_empty() {
                    continue 'item_stack;
                }
                if input.match_item(slot.item) {
                    if input_stack.is_some() {
                        return None;
                    }
                    input_stack = Some(slot);
                } else if material.match_item(slot.item) {
                    material_count += 1;
                } else {
                    return None;
                }
            }
            if input_stack.is_none()
                || material_count < usize::from(*material_count_min)
                || material_count > usize::from(*material_count_max)
            {
                return None;
            }
            let count = if *add_material_count_to_result {
                result.count.saturating_add(material_count as u8)
            } else {
                result.count
            };
            let components = input_stack.and_then(|stack| {
                let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
                stack.write_item_stack(&mut nbt);
                nbt.get_compound("components").cloned()
            });
            Some(RecipeResult {
                recipe_id: canonicalize_recipe_id(recipe_id),
                item_id: result.id.to_string(),
                count,
                components,
            })
        }
        GenericRecipe::Vanilla(CraftingRecipeTypes::CraftingDecoratedPot { recipe_id, .. }) => {
            if count != 4 || inventory.get_width() != 3 || inventory.get_height() != 3 {
                return None;
            }
            let mut decorations = Vec::with_capacity(4);
            for position in [1usize, 3, 5, 7] {
                let slot = inventory.get_stack(position).await;
                if slot.is_empty()
                    || !slot
                        .item
                        .has_tag(&tag::Item::MINECRAFT_DECORATED_POT_INGREDIENTS)
                {
                    return None;
                }
                decorations.push(slot.item.registry_key);
            }
            let mut components = pumpkin_nbt::compound::NbtCompound::new();
            components.put_list(
                "minecraft:pot_decorations",
                decorations
                    .into_iter()
                    .map(|item| {
                        pumpkin_nbt::tag::NbtTag::String(
                            format!("minecraft:{item}").into_boxed_str(),
                        )
                    })
                    .collect(),
            );
            Some(RecipeResult {
                recipe_id: canonicalize_recipe_id(recipe_id),
                item_id: "minecraft:decorated_pot".to_string(),
                count: 1,
                components: Some(components),
            })
        }
        GenericRecipe::Dynamic(OwnedCraftingRecipe::Shaped {
            recipe_id,
            pattern,
            key,
            result,
            ..
        }) => {
            #[allow(clippy::redundant_closure_for_method_calls)]
            if pattern.len() != input_height
                || pattern.first().map_or(0, |f| f.len()) != input_width
            {
                return None;
            }
            if count
                != pattern
                    .iter()
                    .map(|l| l.chars().filter(|c| *c != ' ').count())
                    .sum::<usize>()
            {
                return None;
            }
            let x_offset = top_x;
            let y_offset = top_y;
            let mut matched = true;
            'outer: for (y, row_str) in pattern.iter().enumerate() {
                for (x, current_key) in row_str.chars().enumerate() {
                    let slot = inventory
                        .get_stack((y + y_offset) * inventory.get_width() + (x + x_offset))
                        .await;
                    let slot_index = (y + y_offset) * inventory.get_width() + (x + x_offset);
                    let slot = if inventory.is_slot_enabled(slot_index) {
                        slot
                    } else {
                        ItemStack::EMPTY.clone()
                    };
                    if current_key == ' ' {
                        if !slot.is_empty() {
                            matched = false;
                            break 'outer;
                        }
                        continue;
                    }
                    let Some(ingredient) =
                        key.iter().find(|(k, _)| *k == current_key).map(|(_, v)| v)
                    else {
                        matched = false;
                        break 'outer;
                    };
                    if !ingredient.match_item(slot.item) {
                        matched = false;
                        break 'outer;
                    }
                }
            }
            if !matched
                && !is_symmetrical_horizontally(
                    &pattern.iter().map(String::as_str).collect::<Vec<_>>(),
                )
            {
                matched = true;
                'outer: for (y, row_str) in pattern.iter().enumerate() {
                    for (x, current_key) in row_str.chars().enumerate() {
                        let slot_index = (y + y_offset) * inventory.get_width()
                            + (x_offset + input_width - 1 - x);
                        let slot = inventory.get_stack(slot_index).await;
                        let slot = if inventory.is_slot_enabled(slot_index) {
                            slot
                        } else {
                            ItemStack::EMPTY.clone()
                        };
                        if current_key == ' ' {
                            if !slot.is_empty() {
                                matched = false;
                                break 'outer;
                            }
                            continue;
                        }
                        let Some(ingredient) =
                            key.iter().find(|(k, _)| *k == current_key).map(|(_, v)| v)
                        else {
                            matched = false;
                            break 'outer;
                        };
                        if !ingredient.match_item(slot.item) {
                            matched = false;
                            break 'outer;
                        }
                    }
                }
            }
            matched.then_some(RecipeResult {
                recipe_id: canonicalize_recipe_id(recipe_id),
                item_id: result.item_id.clone(),
                count: result.count,
                components: result.components.clone(),
            })
        }
        GenericRecipe::Dynamic(OwnedCraftingRecipe::Shapeless {
            recipe_id,
            ingredients,
            result,
            ..
        }) => {
            if count != ingredients.len() {
                return None;
            }
            let mut ingredient_used = vec![false; ingredients.len()];
            'next_slot: for i in 0..inventory.size() {
                if !inventory.is_slot_enabled(i) {
                    continue 'next_slot;
                }
                let slot = inventory.get_stack(i).await;
                if slot.is_empty() {
                    continue 'next_slot;
                }
                for i in 0..ingredients.len() {
                    if !ingredient_used[i] && ingredients[i].match_item(slot.item) {
                        ingredient_used[i] = true;
                        continue 'next_slot;
                    }
                }
                return None;
            }
            Some(RecipeResult {
                recipe_id: canonicalize_recipe_id(recipe_id),
                item_id: result.item_id.clone(),
                count: result.count,
                components: result.components.clone(),
            })
        }
        _ => None,
    }
}

impl ResultSlot {
    pub fn new(
        inventory: Arc<dyn RecipeInputInventory>,
        provider: Option<Arc<dyn RecipeProvider>>,
    ) -> Self {
        Self {
            inventory,
            id: AtomicU8::new(0),
            result: Arc::new(Mutex::new(ItemStack::EMPTY.clone())),
            recipe_provider: provider,
        }
    }

    async fn match_recipe(&self) -> Option<RecipeResult> {
        find_crafting_result(&*self.inventory, self.recipe_provider.as_deref()).await
    }

    /// Returns the exact stack left behind when one ingredient is consumed.
    ///
    /// New component-based items carry a `use_remainder` template.  The
    /// generated legacy table remains necessary for data generated before
    /// that component was emitted, so it is intentionally a fallback rather
    /// than a second competing source of truth.
    fn recipe_remainder_for(input: &ItemStack) -> Option<ItemStack> {
        input
            .get_data_component::<UseRemainderImpl>()
            .map(|template| ItemStack::new(template.count, template.remainder))
            .or_else(|| {
                pumpkin_data::recipe_remainder::get_recipe_remainder_id(input.item.id)
                    .and_then(pumpkin_data::item::Item::from_id)
                    .map(|item| ItemStack::new(1, item))
            })
    }

    async fn refill_output(&self) -> ItemStack {
        let result = if let Some(matched) = self.match_recipe().await {
            let key = matched
                .item_id
                .strip_prefix("minecraft:")
                .unwrap_or(&matched.item_id);
            let item = pumpkin_data::item::Item::from_registry_key(key)
                .unwrap_or(&pumpkin_data::item::Item::AIR);
            let mut compound = pumpkin_nbt::compound::NbtCompound::new();
            compound.put_string("id", format!("minecraft:{}", item.registry_key));
            compound.put_int("count", i32::from(matched.count));
            if let Some(components) = matched.components {
                compound.put_compound("components", components);
            }
            pumpkin_data::item_stack::ItemStack::read_item_stack(&compound)
                .unwrap_or_else(|| ItemStack::new(matched.count, item))
        } else {
            ItemStack::EMPTY.clone()
        };
        *self.result.lock().await = result.clone();
        result
    }
}

impl Slot for ResultSlot {
    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.inventory.clone()
    }
    fn get_index(&self) -> usize {
        999
    }
    fn set_id(&self, id: usize) {
        self.id.store(id as u8, Ordering::Relaxed);
    }
    fn on_quick_move_crafted(
        &self,
        _stack: ItemStack,
        _stack_prev: ItemStack,
    ) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.refill_output().await;
        })
    }
    fn on_take_item<'a>(
        &'a self,
        player: &'a dyn InventoryPlayer,
        stack: &'a ItemStack,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            player
                .increment_stat(
                    StatisticCategory::Crafted,
                    stack.item.id as i32,
                    stack.item_count as i32,
                )
                .await;
            for i in 0..self.inventory.size() {
                let input = self.inventory.get_stack(i).await;
                if input.is_empty() {
                    continue;
                }
                let remainder = Self::recipe_remainder_for(&input);
                self.inventory.remove_stack_specific(i, 1).await;
                if let Some(mut remainder) = remainder {
                    let current = self.inventory.get_stack(i).await;
                    if current.is_empty() {
                        let placed = remainder.split(remainder.get_max_stack_size());
                        self.inventory.set_stack(i, placed).await;
                        if !remainder.is_empty() {
                            offer_or_drop_stack(player, remainder).await;
                        }
                    } else if current.are_items_and_components_equal(&remainder)
                        && current.item_count < current.get_max_stack_size()
                    {
                        let mut merged = current;
                        let available = merged
                            .get_max_stack_size()
                            .saturating_sub(merged.item_count);
                        let merged_count = remainder.item_count.min(available);
                        merged.increment(merged_count);
                        remainder.decrement(merged_count);
                        self.inventory.set_stack(i, merged).await;
                        if !remainder.is_empty() {
                            offer_or_drop_stack(player, remainder).await;
                        }
                    } else {
                        // Vanilla never silently deletes a recipe remainder.  If the
                        // source slot is occupied by an incompatible stack, route it
                        // through the same inventory-then-drop path used by all other
                        // container handlers.
                        offer_or_drop_stack(player, remainder).await;
                    }
                }
            }
            self.mark_dirty().await;
        })
    }
    fn can_take_items<'a>(&'a self, player: &'a dyn InventoryPlayer) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            let Some(recipe) = self.match_recipe().await else {
                return false;
            };
            player.can_craft_recipe(&recipe.recipe_id).await
        })
    }
    fn can_insert(&self, _stack: &ItemStack) -> BoxFuture<'_, bool> {
        Box::pin(async move { false })
    }
    fn get_stack(&self) -> BoxFuture<'_, ItemStack> {
        Box::pin(async move { self.result.lock().await.clone() })
    }
    fn get_cloned_stack(&self) -> BoxFuture<'_, ItemStack> {
        Box::pin(async move { self.result.lock().await.clone() })
    }
    fn has_stack(&self) -> BoxFuture<'_, bool> {
        Box::pin(async move { !self.result.lock().await.is_empty() })
    }
    fn set_stack(&self, _stack: ItemStack) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.refill_output().await;
        })
    }
    fn set_stack_no_callbacks(&self, stack: ItemStack) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            // The result inventory is a virtual slot (index 999), so it must
            // never be routed through Inventory::set_stack.  In particular,
            // closing a menu clears the cached result without re-running the
            // recipe matcher or leaving a stale output visible to the client.
            *self.result.lock().await = stack;
            self.inventory.mark_dirty();
        })
    }
    fn set_stack_prev(&self, _stack: ItemStack, _previous_stack: ItemStack) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.refill_output().await;
        })
    }
    fn mark_dirty(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.inventory.mark_dirty();
        })
    }
    fn get_max_item_count(&self) -> BoxFuture<'_, u8> {
        Box::pin(async move {
            let mut count = u8::MAX;
            for i in 0..self.inventory.size() {
                let slot = self.inventory.get_stack(i).await;
                if !slot.is_empty() {
                    count = count.min(slot.item_count);
                }
            }
            count
        })
    }
    fn take_stack(&self, _amount: u8) -> BoxFuture<'_, ItemStack> {
        Box::pin(async move {
            if self.has_stack().await {
                self.result.lock().await.clone()
            } else {
                ItemStack::EMPTY.clone()
            }
        })
    }
}

impl ScreenHandlerListener for ResultSlot {
    fn on_slot_update<'a>(
        &'a self,
        screen_handler: &'a ScreenHandlerBehaviour,
        slot: u8,
        _stack: ItemStack,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            if (0..=(self.inventory.get_width() * self.inventory.get_height()))
                .contains(&(slot as usize))
            {
                let result = self.refill_output().await;
                let next_revision = screen_handler.next_revision();
                if let Some(sync_handler) = screen_handler.sync_handler.as_ref() {
                    sync_handler
                        .update_slot(screen_handler, 0, &result, next_revision)
                        .await;
                }
            }
        })
    }
}

pub trait CraftingScreenHandler<I: RecipeInputInventory>:
    RecipeFinderScreenHandler + ScreenHandler
{
    fn add_recipe_slots<'a>(
        &'a mut self,
        crafing_inventory: Arc<dyn RecipeInputInventory>,
        provider: Option<Arc<dyn RecipeProvider>>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let result_slot = Arc::new(ResultSlot::new(crafing_inventory.clone(), provider));
            self.add_slot(result_slot.clone());
            let width = crafing_inventory.get_width();
            let height = crafing_inventory.get_height();
            for i in 0..width {
                for j in 0..height {
                    let input_slot = NormalSlot::new(crafing_inventory.clone(), j + i * width);
                    self.add_slot(Arc::new(input_slot));
                }
            }
            self.add_listener(result_slot).await;
        })
    }
}

pub struct CraftingTableScreenHandler {
    behaviour: ScreenHandlerBehaviour,
    crafting_inventory: Arc<dyn RecipeInputInventory>,
}

impl CraftingTableScreenHandler {
    pub async fn new(
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        provider: Option<Arc<dyn RecipeProvider>>,
    ) -> Self {
        let crafting_inventory: Arc<dyn RecipeInputInventory> =
            Arc::new(CraftingInventory::new(3, 3));
        let mut crafting_table_handler = Self {
            behaviour: ScreenHandlerBehaviour::new(sync_id, Some(WindowType::Crafting)),
            crafting_inventory: crafting_inventory.clone(),
        };
        crafting_table_handler
            .add_recipe_slots(crafting_inventory, provider)
            .await;
        let player_inventory: Arc<dyn Inventory> = player_inventory.clone();
        crafting_table_handler.add_player_slots(&player_inventory);
        crafting_table_handler
    }
}

impl RecipeFinderScreenHandler for CraftingTableScreenHandler {}

impl ScreenHandler for CraftingTableScreenHandler {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn get_behaviour(&self) -> &ScreenHandlerBehaviour {
        &self.behaviour
    }
    fn get_behaviour_mut(&mut self) -> &mut ScreenHandlerBehaviour {
        &mut self.behaviour
    }
    fn on_closed<'a>(&'a mut self, player: &'a dyn InventoryPlayer) -> ScreenHandlerFuture<'a, ()> {
        Box::pin(async move {
            self.default_on_closed(player).await;
            self.get_behaviour().slots[0]
                .set_stack_no_callbacks(ItemStack::EMPTY.clone())
                .await;
            self.drop_inventory(player, self.crafting_inventory.clone())
                .await;
        })
    }
    fn quick_move<'a>(
        &'a mut self,
        player: &'a dyn InventoryPlayer,
        slot_index: i32,
    ) -> ItemStackFuture<'a> {
        Box::pin(async move {
            let slot = self.get_behaviour().slots[slot_index as usize].clone();
            if slot.has_stack().await {
                let mut slot_stack = slot.get_stack().await;
                let stack_prev = slot_stack.clone();
                if slot_index == 0 {
                    if !self.insert_item(&mut slot_stack, 10, 46, true).await {
                        return ItemStack::EMPTY.clone();
                    }
                } else if (1..=9).contains(&slot_index) {
                    if !self.insert_item(&mut slot_stack, 10, 46, false).await {
                        return ItemStack::EMPTY.clone();
                    }
                } else if (10..46).contains(&slot_index) {
                    if !self.insert_item(&mut slot_stack, 1, 10, false).await {
                        if slot_index < 37 {
                            if !self.insert_item(&mut slot_stack, 37, 46, false).await {
                                return ItemStack::EMPTY.clone();
                            }
                        } else if !self.insert_item(&mut slot_stack, 10, 37, false).await {
                            return ItemStack::EMPTY.clone();
                        }
                    }
                } else if !self.insert_item(&mut slot_stack, 10, 46, false).await {
                    return ItemStack::EMPTY.clone();
                }
                let stack = slot_stack.clone();
                drop(slot_stack);
                if stack.is_empty() {
                    slot.set_stack_prev(ItemStack::EMPTY.clone(), stack_prev.clone())
                        .await;
                } else {
                    slot.mark_dirty().await;
                }
                if stack.item_count == stack_prev.item_count {
                    return ItemStack::EMPTY.clone();
                }

                let mut taken_stack = stack_prev.clone();
                taken_stack.set_count(stack_prev.item_count - stack.item_count);
                slot.on_take_item(player, &taken_stack).await;

                if slot_index == 0 {
                    slot.on_quick_move_crafted(stack.clone(), stack_prev.clone())
                        .await;
                    if !stack.is_empty() {
                        player.drop_item(stack, false).await;
                    }
                }
                return stack_prev;
            }
            ItemStack::EMPTY.clone()
        })
    }
}

impl CraftingScreenHandler<CraftingInventory> for CraftingTableScreenHandler {}

#[cfg(test)]
mod tests {
    use super::{ResultSlot, find_crafting_result};
    use crate::crafting::crafting_inventory::CraftingInventory;
    use crate::crafting::recipe_provider::RecipeProvider;
    use pumpkin_data::item::Item;
    use pumpkin_data::item_stack::ItemStack;
    use pumpkin_data::recipes::RecipeCategoryTypes;
    use pumpkin_protocol::codec::recipe::{
        DynamicRecipe, OwnedCraftingRecipe, OwnedRecipeIngredient, OwnedRecipeResult,
    };
    use pumpkin_world::inventory::Inventory;

    #[tokio::test]
    async fn two_planks_match_stick_recipe_in_player_grid() {
        let inventory = CraftingInventory::new(2, 2);
        inventory
            .set_stack(0, ItemStack::new(1, &Item::OAK_PLANKS))
            .await;
        inventory
            .set_stack(2, ItemStack::new(1, &Item::OAK_PLANKS))
            .await;

        let result = find_crafting_result(&inventory, None).await;
        assert_eq!(
            result.as_ref().map(|value| value.item_id.as_str()),
            Some("minecraft:stick")
        );
        assert_eq!(result.map(|value| value.count), Some(4));
    }

    #[test]
    fn recipe_remainder_uses_item_component_template() {
        let milk_bucket = ItemStack::new(1, &Item::MILK_BUCKET);
        let remainder = ResultSlot::recipe_remainder_for(&milk_bucket)
            .expect("milk bucket must leave a bucket");
        assert_eq!(remainder.item.id, Item::BUCKET.id);
        assert_eq!(remainder.item_count, 1);

        let honey_bottle = ItemStack::new(1, &Item::HONEY_BOTTLE);
        let remainder = ResultSlot::recipe_remainder_for(&honey_bottle)
            .expect("honey bottle must leave a glass bottle");
        assert_eq!(remainder.item.id, Item::GLASS_BOTTLE.id);
    }

    #[tokio::test]
    async fn dynamic_shaped_recipe_matches_mirrored_pattern() {
        let inventory = CraftingInventory::new(2, 1);
        inventory
            .set_stack(0, ItemStack::new(1, &Item::STICK))
            .await;
        inventory
            .set_stack(1, ItemStack::new(1, &Item::OAK_PLANKS))
            .await;

        let provider = TestRecipeProvider {
            recipes: vec![DynamicRecipe::Crafting(OwnedCraftingRecipe::Shaped {
                recipe_id: "example:mirrored".to_owned(),
                category: RecipeCategoryTypes::Misc,
                group: None,
                show_notification: true,
                key: vec![
                    (
                        'a',
                        OwnedRecipeIngredient::Simple("minecraft:oak_planks".to_owned()),
                    ),
                    (
                        'b',
                        OwnedRecipeIngredient::Simple("minecraft:stick".to_owned()),
                    ),
                ],
                pattern: vec!["ab".to_owned()],
                result: OwnedRecipeResult {
                    item_id: "minecraft:crafting_table".to_owned(),
                    count: 1,
                    components: None,
                },
            })],
        };

        let result = find_crafting_result(&inventory, Some(&provider)).await;
        assert_eq!(
            result.as_ref().map(|value| value.recipe_id.as_str()),
            Some("example:mirrored")
        );
        assert_eq!(
            result.as_ref().map(|value| value.item_id.as_str()),
            Some("minecraft:crafting_table")
        );
    }

    #[tokio::test]
    async fn decorated_pot_carries_the_four_sherd_components() {
        let inventory = CraftingInventory::new(3, 3);
        for (slot, item) in [
            (1, &Item::ANGLER_POTTERY_SHERD),
            (3, &Item::ARCHER_POTTERY_SHERD),
            (5, &Item::BRICK),
            (7, &Item::BLADE_POTTERY_SHERD),
        ] {
            inventory.set_stack(slot, ItemStack::new(1, item)).await;
        }

        let result = find_crafting_result(&inventory, None)
            .await
            .expect("decorated pot should match");
        let components = result.components.expect("pot decorations component");
        let sides = components
            .get_list("minecraft:pot_decorations")
            .expect("four ordered sides");
        let names: Vec<_> = sides
            .iter()
            .map(|value| value.extract_string().expect("string item id").to_string())
            .collect();
        assert_eq!(
            names,
            vec![
                "minecraft:angler_pottery_sherd",
                "minecraft:archer_pottery_sherd",
                "minecraft:brick",
                "minecraft:blade_pottery_sherd",
            ]
        );
    }

    #[tokio::test]
    async fn transmute_copies_input_components_and_supports_material_bounds() {
        let inventory = CraftingInventory::new(3, 3);
        let mut filled_map = ItemStack::new(1, &Item::FILLED_MAP);
        filled_map.set_custom_data("test", "marker", pumpkin_nbt::tag::NbtTag::Byte(1));
        inventory.set_stack(0, filled_map).await;
        inventory.set_stack(1, ItemStack::new(1, &Item::MAP)).await;
        inventory.set_stack(2, ItemStack::new(1, &Item::MAP)).await;

        let result = find_crafting_result(&inventory, None)
            .await
            .expect("map cloning should match two materials");
        assert_eq!(result.recipe_id, "minecraft:map_cloning");
        assert_eq!(result.count, 3);
        let components = result.components.expect("input components are copied");
        assert_eq!(
            components
                .get_compound("minecraft:custom_data")
                .and_then(|custom| custom.get_compound("test"))
                .and_then(|test| test.get_byte("marker")),
            Some(1)
        );
    }

    struct TestRecipeProvider {
        recipes: Vec<DynamicRecipe>,
    }

    impl RecipeProvider for TestRecipeProvider {
        fn get_dynamic_recipes(&self) -> crate::slot::BoxFuture<'_, Vec<DynamicRecipe>> {
            Box::pin(async { self.recipes.clone() })
        }
    }
}
