//! Screen handler module.
//!
//! This module defines the core screen handler system for container UIs.
//! A screen handler manages the server-side state of a container interface,
//! handling slot layout, click processing, item transfer, and synchronization
//! with the client.
//!
//! # Core Components
//!
//! - [`ScreenHandler`] - The main trait for container screen handlers
//! - [`ScreenHandlerBehaviour`] - Shared state for all screen handlers
//! - [`InventoryPlayer`] - Interface for player interactions with containers
//! - [`ScreenProperty`] - Container UI properties (progress bars, etc.)
//!
//! # Screen Handler Lifecycle
//!
//! 1. Creation - Screen handler is created with slots and sync ID
//! 2. Opening - Player opens the container, sync handler attaches
//! 3. Interaction - Click packets are processed, items move between slots
//! 4. Closing - Container closes, cursor item is dropped/given to player
//!
//! # Slot Indexing
//!
//! Slots are indexed from 0 within each screen handler. Special values:
//! - `-1` - Cursor slot (held item)
//! - `-999` - Outside inventory (drop to world)

use crate::{
    container_click::MouseClick,
    player::player_inventory::PlayerInventory,
    slot::{NormalSlot, Slot},
    sync_handler::{SyncHandler, TrackedStack},
};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::{
    data_component_impl::EquipmentSlot, screen::WindowType, statistic::StatisticCategory,
};
use pumpkin_protocol::{
    codec::item_stack_seralizer::OptionalItemStackHash,
    java::{
        client::play::{
            CSetContainerContent, CSetContainerProperty, CSetContainerSlot, CSetCursorItem,
            CSetPlayerInventory, CSetSelectedSlot,
        },
        server::play::SlotActionType,
    },
};
use pumpkin_util::text::TextComponent;
use pumpkin_world::{
    block::entities::PropertyDelegate,
    inventory::{ComparableInventory, Inventory},
};
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::{any::Any, collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use tracing::warn;

/// Slot index indicating a click outside the inventory.
const SLOT_INDEX_OUTSIDE: i32 = -999;

#[inline]
const fn is_valid_slot_index(slot: i32, slot_count: usize) -> bool {
    slot == -1 || slot == SLOT_INDEX_OUTSIDE || (slot >= 0 && (slot as usize) < slot_count)
}

/// One deterministic result of a vanilla quick-craft distribution.
struct QuickCraftPlacement {
    slot: u32,
    stack: ItemStack,
    inserted: u8,
}

/// Computes the quick-craft writes before any slot is mutated.
///
/// Keeping this calculation side-effect free is important: a malformed click
/// must never partially distribute the carried stack.  `slots` contains the
/// selected slots in protocol order, their current stacks, and each slot's
/// effective max count.  Invalid/incompatible slots are represented by their
/// snapshot and are skipped exactly like `AbstractContainerMenu` does.
fn calculate_quick_craft_plan(
    source: &ItemStack,
    slots: &[(u32, ItemStack, u8)],
    quick_craft_type: u8,
) -> (Vec<QuickCraftPlacement>, u8) {
    if source.is_empty() || slots.is_empty() || quick_craft_type > 2 {
        return (Vec::new(), source.item_count);
    }

    let place_count = match quick_craft_type {
        0 => source.item_count / slots.len() as u8,
        1 => 1,
        // Creative quick-craft uses the item's maximum, not the current
        // carried count, and the cursor is cleared after the operation.
        2 => source.get_max_stack_size(),
        _ => return (Vec::new(), source.item_count),
    };
    let mut remaining = source.item_count;
    let mut placements = Vec::with_capacity(slots.len());

    for (slot, current, slot_max) in slots {
        if !current.is_empty() && !current.are_items_and_components_equal(source) {
            continue;
        }

        let max_count = source.get_max_stack_size().min(*slot_max);
        let available = max_count.saturating_sub(current.item_count);
        let inserted = place_count.min(available);
        if inserted == 0 {
            continue;
        }

        let mut next = if current.is_empty() {
            source.copy_with_count(0)
        } else {
            current.clone()
        };
        next.item_count = next.item_count.saturating_add(inserted);
        placements.push(QuickCraftPlacement {
            slot: *slot,
            stack: next,
            inserted,
        });

        if quick_craft_type != 2 {
            remaining = remaining.saturating_sub(inserted);
        }
    }

    if quick_craft_type == 2 {
        remaining = 0;
    }
    (placements, remaining)
}

/// A tracked property for container UI elements.
///
/// Properties are used to synchronize UI state like furnace progress bars,
/// enchantment levels, and other visual indicators between server and client.
pub struct ScreenProperty {
    old_value: i32,
    index: u8,
    value: Arc<dyn PropertyDelegate>,
}

impl ScreenProperty {
    /// Creates a new screen property.
    ///
    /// # Arguments
    /// - `value` - The property delegate that holds the actual value
    /// - `index` - The property index for multi-value delegates
    pub fn new(value: Arc<dyn PropertyDelegate>, index: u8) -> Self {
        Self {
            old_value: value.get_property(i32::from(index)),
            index,
            value,
        }
    }

    /// Gets the current property value.
    #[must_use]
    pub fn get(&self) -> i32 {
        self.value.get_property(i32::from(self.index))
    }

    /// Sets the property value.
    pub fn set(&mut self, value: i32) {
        self.value.set_property(i32::from(self.index), value);
    }

    /// Checks if the value has changed since the last check.
    ///
    /// Updates the old value to the current value.
    pub fn has_changed(&mut self) -> bool {
        let value = self.get();
        let has_changed = !value.eq(&self.old_value);
        self.old_value = value;
        has_changed
    }
}

/// Type alias for async player operations.
/// Type alias for async player operations.
pub type PlayerFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Interface for player interactions with containers.
///
/// This trait abstracts the player's ability to:
/// - Drop items into the world
/// - Receive inventory packets
/// - Change equipment
/// - Receive experience
///
/// Implementors are typically player entities that can open containers.
pub trait InventoryPlayer: Send + Sync {
    fn as_any(&self) -> &dyn std::any::Any;
    /// Drops an item into the world.
    ///
    /// # Arguments
    /// - `item` - The item to drop
    /// - `retain_ownership` - If true, the player keeps ownership (for pickup delay)
    fn drop_item(&self, item: ItemStack, retain_ownership: bool) -> PlayerFuture<'_, ()>;

    /// Gets the player's inventory.
    fn get_inventory(&self) -> Arc<PlayerInventory>;

    /// Checks if the player has infinite materials (creative mode).
    fn has_infinite_materials(&self) -> bool;

    /// Checks if the player is in creative mode.
    fn is_creative(&self) -> bool;

    /// Gets the player's experience level.
    fn experience_level(&self) -> i32;

    /// Adds or removes experience levels.
    fn add_experience_levels(&self, levels: i32) -> PlayerFuture<'_, ()>;

    /// Gets the player's enchantment seed.
    fn enchantment_seed(&self) -> i32;

    /// Sets the player's enchantment seed.
    fn set_enchantment_seed(&self, seed: i32) -> PlayerFuture<'_, ()>;

    /// Sends a full container content packet.
    fn enqueue_inventory_packet<'a>(
        &'a self,
        packet: &'a CSetContainerContent,
    ) -> PlayerFuture<'a, ()>;

    /// Sends a single slot update packet.
    fn enqueue_slot_packet<'a>(
        &'a self,
        packet: &'a CSetContainerSlot,
        window_type: Option<WindowType>,
        total_slots: usize,
    ) -> PlayerFuture<'a, ()>;

    /// Sends a cursor item update packet.
    fn enqueue_cursor_packet<'a>(&'a self, packet: &'a CSetCursorItem) -> PlayerFuture<'a, ()>;

    /// Sends a property update packet.
    fn enqueue_property_packet<'a>(
        &'a self,
        packet: &'a CSetContainerProperty,
    ) -> PlayerFuture<'a, ()>;

    /// Sends a player inventory slot update.
    fn enqueue_slot_set_packet<'a>(
        &'a self,
        packet: &'a CSetPlayerInventory,
    ) -> PlayerFuture<'a, ()>;

    /// Sends a selected slot update.
    fn enqueue_set_held_item_packet<'a>(
        &'a self,
        packet: &'a CSetSelectedSlot,
    ) -> PlayerFuture<'a, ()>;

    /// Sends an equipment change packet.
    fn enqueue_equipment_change<'a>(
        &'a self,
        slot: &'a EquipmentSlot,
        stack: &'a ItemStack,
    ) -> PlayerFuture<'a, ()>;

    /// Awards experience points to the player (used for furnace smelting, etc.)
    fn award_experience(&self, amount: i32) -> PlayerFuture<'_, ()>;

    /// Increments a statistic for the player.
    fn increment_stat(
        &self,
        category: StatisticCategory,
        stat_id: i32,
        amount: i32,
    ) -> PlayerFuture<'_, ()>;

    /// Checks the server-side recipe-book rule before a result slot is taken.
    ///
    /// The inventory crate deliberately does not depend on the Pumpkin world
    /// type.  The player adapter supplies the authoritative `limited_crafting`
    /// and learned-recipe state through this small async capability instead.
    fn can_craft_recipe<'a>(&'a self, recipe_id: &'a str) -> PlayerFuture<'a, bool>;
}

/// Gives a stack to the player or drops it if inventory is full.
///
/// Tries to insert the stack into the player's inventory first,
/// and drops it in the world if there's no room.
pub async fn offer_or_drop_stack(player: &dyn InventoryPlayer, stack: ItemStack) {
    // TODO: Super weird disconnect logic in vanilla, investigate this later
    player
        .get_inventory()
        .offer_or_drop_stack(stack, player)
        .await;
}

/// Maps the player-inventory menu indices that represent entity equipment.
///
/// These indices are stable Java protocol positions for the player inventory
/// screen. Other container menus do not expose the off-hand slot, so callers
/// must only use this mapping for a player inventory layout.
pub(crate) const fn player_screen_equipment_slot(slot_index: i32) -> Option<EquipmentSlot> {
    match slot_index {
        5 => Some(EquipmentSlot::HEAD),
        6 => Some(EquipmentSlot::CHEST),
        7 => Some(EquipmentSlot::LEGS),
        8 => Some(EquipmentSlot::FEET),
        45 => Some(EquipmentSlot::OFF_HAND),
        _ => None,
    }
}

/// Type alias for async screen handler operations.
pub type ScreenHandlerFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Future type that returns an `ItemStack` (used by `quick_move`).
pub type ItemStackFuture<'a> = ScreenHandlerFuture<'a, ItemStack>;

/// Future type that returns an optional slot index.
pub type OptionUsizeFuture<'a> = ScreenHandlerFuture<'a, Option<usize>>;

/// The main trait for container screen handlers.
///
/// Screen handlers manage the server-side state of container UIs like chests,
/// furnaces, crafting tables, etc. They handle:
/// - Slot layout and management
/// - Click processing
/// - Item transfer logic (shift-click)
/// - Client synchronization
///
/// # Implementation
///
/// Implementors must provide:
/// - [`get_behaviour`](ScreenHandler::get_behaviour) and [`get_behaviour_mut`](ScreenHandler::get_behaviour_mut)
/// - [`quick_move`](ScreenHandler::quick_move) for shift-click behavior
/// - [`as_any`](ScreenHandler::as_any) for downcasting
// ScreenHandler.java
// TODO: Fully implement this
pub trait ScreenHandler: Send + Sync {
    // --- Synchronous Methods ---

    /// Gets the window type for this screen handler.
    fn window_type(&self) -> Option<WindowType> {
        self.get_behaviour().window_type
    }

    /// Returns this screen handler as an Any reference.
    fn as_any(&self) -> &dyn Any;

    /// Returns this screen handler as a mutable Any reference.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Gets the sync ID for this screen handler.
    fn sync_id(&self) -> u8 {
        self.get_behaviour().sync_id
    }

    /// Checks if the player can use this container.
    fn can_use(&self, _player: &dyn InventoryPlayer) -> bool {
        true
    }

    /// Gets a reference to the screen handler behaviour.
    fn get_behaviour(&self) -> &ScreenHandlerBehaviour;

    /// Gets a mutable reference to the screen handler behaviour.
    fn get_behaviour_mut(&mut self) -> &mut ScreenHandlerBehaviour;

    /// Adds a slot to this screen handler.
    ///
    /// Assigns an ID and sets up tracking for the slot.
    fn add_slot(&mut self, slot: Arc<dyn Slot>) -> Arc<dyn Slot> {
        let behaviour = self.get_behaviour_mut();
        slot.set_id(behaviour.slots.len());
        behaviour.slots.push(slot.clone());
        behaviour.tracked_stacks.push(ItemStack::EMPTY.clone());
        behaviour.previous_tracked_stacks.push(TrackedStack::EMPTY);

        slot
    }

    /// Adds hotbar slots (0-8) from the player inventory.
    fn add_player_hotbar_slots(&mut self, player_inventory: &Arc<dyn Inventory>) {
        for i in 0..9 {
            self.add_slot(Arc::new(NormalSlot::new(player_inventory.clone(), i)));
        }
    }

    /// Adds main inventory slots (9-35) from the player inventory.
    fn add_player_inventory_slots(&mut self, player_inventory: &Arc<dyn Inventory>) {
        for i in 0..3 {
            for j in 0..9 {
                self.add_slot(Arc::new(NormalSlot::new(
                    player_inventory.clone(),
                    j + (i + 1) * 9,
                )));
            }
        }
    }

    /// Adds all player inventory slots (main + hotbar).
    fn add_player_slots(&mut self, player_inventory: &Arc<dyn Inventory>) {
        self.add_player_inventory_slots(player_inventory);
        self.add_player_hotbar_slots(player_inventory);
    }

    /// Records a received hash for a slot (for sync tracking).
    fn set_received_hash(&mut self, slot: usize, hash: OptionalItemStackHash) {
        let behaviour = self.get_behaviour_mut();
        if slot < behaviour.previous_tracked_stacks.len() {
            behaviour.previous_tracked_stacks[slot].set_received_hash(hash);
        } else {
            warn!(
                "Incorrect slot index: {} available slots: {}",
                slot,
                behaviour.previous_tracked_stacks.len()
            );
        }
    }

    /// Records a received stack for a slot (for sync tracking).
    fn set_received_stack(&mut self, slot: usize, stack: ItemStack) {
        let behaviour = self.get_behaviour_mut();
        if let Some(tracked) = behaviour.previous_tracked_stacks.get_mut(slot) {
            tracked.set_received_stack(stack);
        } else {
            warn!(
                "Incorrect received stack slot: {} available slots: {}",
                slot,
                behaviour.previous_tracked_stacks.len()
            );
        }
    }

    /// Records a received cursor hash (for sync tracking).
    fn set_received_cursor_hash(&mut self, hash: OptionalItemStackHash) {
        let behaviour = self.get_behaviour_mut();
        behaviour.previous_cursor_stack.set_received_hash(hash);
    }

    /// Adds a property to track.
    fn add_property(&mut self, property: ScreenProperty) {
        let behaviour = self.get_behaviour_mut();
        behaviour.properties.push(property);
        behaviour.tracked_property_values.push(0);
    }

    /// Adds multiple properties to track.
    fn add_properties(&mut self, properties: Vec<ScreenProperty>) {
        for property in properties {
            self.add_property(property);
        }
    }

    // --- Asynchronous Methods ---

    /// Called when the container is closed by the player.
    ///
    /// Default implementation drops the cursor item.
    fn on_closed<'a>(&'a mut self, player: &'a dyn InventoryPlayer) -> ScreenHandlerFuture<'a, ()> {
        Box::pin(async move {
            self.default_on_closed(player).await;
        })
    }

    /// Default close behavior - drops the cursor item.
    fn default_on_closed<'a>(
        &'a mut self,
        player: &'a dyn InventoryPlayer,
    ) -> ScreenHandlerFuture<'a, ()> {
        Box::pin(async move {
            let behaviour = self.get_behaviour_mut();

            // Lock and clone are performed inside the async block
            let mut cursor_stack_lock = behaviour.cursor_stack.lock().await;

            if !cursor_stack_lock.is_empty() {
                offer_or_drop_stack(player, cursor_stack_lock.clone()).await;
                *cursor_stack_lock = ItemStack::EMPTY.clone();
            }
        })
    }

    /// Drops all items from an inventory into the world.
    fn drop_inventory<'a>(
        &'a self,
        player: &'a dyn InventoryPlayer,
        inventory: Arc<dyn Inventory>,
    ) -> ScreenHandlerFuture<'a, ()> {
        Box::pin(async move {
            for i in 0..inventory.size() {
                offer_or_drop_stack(player, inventory.remove_stack(i).await).await;
            }
        })
    }

    /// Copies tracked slot state from another screen handler.
    ///
    /// Used when reopening a container to restore previous state.
    fn copy_shared_slots(
        &mut self,
        other: Arc<Mutex<dyn ScreenHandler>>,
    ) -> ScreenHandlerFuture<'_, ()> {
        Box::pin(async move {
            let mut table: HashMap<ComparableInventory, HashMap<usize, usize>> = HashMap::new();
            let other_binding = other.lock().await;
            let other_behaviour = other_binding.get_behaviour();

            for i in 0..other_behaviour.slots.len() {
                let other_slot = other_behaviour.slots[i].clone();
                let mut hash_map = HashMap::new();
                hash_map.insert(other_slot.get_index(), i);
                table.insert(
                    ComparableInventory(other_slot.get_inventory().clone()),
                    hash_map,
                );
            }

            for i in 0..self.get_behaviour().slots.len() {
                let slot = self.get_behaviour().slots[i].clone();
                let inventory = slot.get_inventory();
                let index = slot.get_index();

                if let Some(hash_map) = table.get(&ComparableInventory(inventory.clone()))
                    && let Some(other_index) = hash_map.get(&index)
                {
                    self.get_behaviour_mut().tracked_stacks[i] =
                        other_behaviour.tracked_stacks[*other_index].clone();
                    self.get_behaviour_mut().previous_tracked_stacks[i] =
                        other_behaviour.previous_tracked_stacks[*other_index].clone();
                }
            }
        })
    }

    /// Synchronizes the full state to the client.
    ///
    /// Captures current slot states and sends a full update packet.
    fn sync_state(&mut self) -> ScreenHandlerFuture<'_, ()> {
        Box::pin(async move {
            let behaviour = self.get_behaviour_mut();
            let mut previous_tracked_stacks = Vec::new();

            for i in 0..behaviour.slots.len() {
                let stack = behaviour.slots[i].get_cloned_stack().await;
                previous_tracked_stacks.push(stack.clone());
                behaviour.previous_tracked_stacks[i].set_received_stack(stack);
            }

            let cursor_stack = behaviour.cursor_stack.lock().await.clone();
            behaviour
                .previous_cursor_stack
                .set_received_stack(cursor_stack.clone());

            for i in 0..behaviour.properties.len() {
                let property_val = behaviour.properties[i].get();
                behaviour.tracked_property_values[i] = property_val;
            }

            let next_revision = behaviour.next_revision();

            if let Some(sync_handler) = behaviour.sync_handler.as_ref() {
                sync_handler
                    .update_state(
                        behaviour,
                        &previous_tracked_stacks,
                        &cursor_stack,
                        behaviour.tracked_property_values.clone(),
                        next_revision,
                    )
                    .await;
            }
        })
    }

    /// Adds a listener for slot and property changes.
    fn add_listener(
        &mut self,
        listener: Arc<dyn ScreenHandlerListener>,
    ) -> ScreenHandlerFuture<'_, ()> {
        Box::pin(async move {
            self.get_behaviour_mut().listeners.push(listener);
            self.send_content_updates().await;
        })
    }

    /// Attaches a sync handler and performs initial sync.
    fn update_sync_handler(
        &mut self,
        sync_handler: Arc<SyncHandler>,
    ) -> ScreenHandlerFuture<'_, ()> {
        Box::pin(async move {
            let behaviour = self.get_behaviour_mut();
            behaviour.sync_handler = Some(sync_handler.clone());
            self.sync_state().await;
        })
    }

    /// Sends all updates to the client.
    ///
    /// Updates tracked slots and properties.
    fn update_to_client(&mut self) -> ScreenHandlerFuture<'_, ()> {
        Box::pin(async move {
            for i in 0..self.get_behaviour().slots.len() {
                let behaviour = self.get_behaviour_mut();
                let slot = behaviour.slots[i].clone();
                let stack = slot.get_cloned_stack().await;
                self.update_tracked_slot(i, stack).await;
            }

            let behaviour = self.get_behaviour_mut();
            let mut prop_vec = vec![];
            for (idx, prop) in behaviour.properties.iter_mut().enumerate() {
                let value = prop.get();
                if prop.has_changed() {
                    prop_vec.push((idx, value));
                }
            }

            for (idx, value) in prop_vec {
                self.update_tracked_properties(idx as i32, value).await;
                self.check_property_updates(idx as i32, value).await;
            }

            self.sync_state().await;
        })
    }

    /// Updates a tracked property value.
    fn update_tracked_properties(&mut self, idx: i32, value: i32) -> ScreenHandlerFuture<'_, ()> {
        Box::pin(async move {
            let behaviour = self.get_behaviour_mut();
            if idx >= 0 && idx < behaviour.tracked_property_values.len() as i32 {
                behaviour.tracked_property_values[idx as usize] = value;
                for listener in &behaviour.listeners {
                    listener
                        .on_property_update(behaviour, idx as u8, value)
                        .await;
                }
            }
        })
    }

    /// Checks if a property needs to be synced to the client.
    fn check_property_updates(&mut self, idx: i32, value: i32) -> ScreenHandlerFuture<'_, ()> {
        Box::pin(async move {
            let behaviour = self.get_behaviour_mut();
            if !behaviour.disable_sync
                && let Some(old_value) = behaviour.tracked_property_values.get(idx as usize)
            {
                let old_value = *old_value;
                if old_value != value {
                    behaviour
                        .tracked_property_values
                        .insert(idx as usize, value);
                    if let Some(ref sync_handler) = behaviour.sync_handler {
                        sync_handler.update_property(behaviour, idx, value).await;
                    }
                }
            }
        })
    }

    /// Updates the tracked state of a slot.
    fn update_tracked_slot(
        &mut self,
        slot: usize,
        stack: ItemStack,
    ) -> ScreenHandlerFuture<'_, ()> {
        Box::pin(async move {
            let behaviour = self.get_behaviour_mut();
            let other_stack = &behaviour.tracked_stacks[slot];
            if !other_stack.are_equal(&stack) {
                behaviour.tracked_stacks[slot] = stack.clone();

                for listener in &behaviour.listeners {
                    listener
                        .on_slot_update(behaviour, slot as u8, stack.clone())
                        .await;
                }
            }
        })
    }

    /// Checks if a slot needs to be synced to the client.
    fn check_slot_updates(&mut self, slot: usize, stack: ItemStack) -> ScreenHandlerFuture<'_, ()> {
        Box::pin(async move {
            let behaviour = self.get_behaviour_mut();
            if !behaviour.disable_sync {
                let prev_stack = &mut behaviour.previous_tracked_stacks[slot];

                if !prev_stack.is_in_sync(&stack) {
                    prev_stack.set_received_stack(stack.clone());
                    let next_revision = behaviour.next_revision();
                    if let Some(sync_handler) = behaviour.sync_handler.as_ref() {
                        sync_handler
                            .update_slot(behaviour, slot, &stack, next_revision)
                            .await;
                    }
                }
            }
        })
    }

    /// Checks if the cursor stack needs to be synced.
    fn check_cursor_stack_updates(&mut self) -> ScreenHandlerFuture<'_, ()> {
        Box::pin(async move {
            let behaviour = self.get_behaviour_mut();
            if !behaviour.disable_sync {
                let cursor_stack = behaviour.cursor_stack.lock().await;
                if !behaviour.previous_cursor_stack.is_in_sync(&cursor_stack) {
                    behaviour
                        .previous_cursor_stack
                        .set_received_stack(cursor_stack.clone());
                    if let Some(sync_handler) = behaviour.sync_handler.as_ref() {
                        sync_handler
                            .update_cursor_stack(behaviour, &cursor_stack)
                            .await;
                    }
                }
            }
        })
    }

    /// Sends all content updates to listeners and sync handler.
    fn send_content_updates(&mut self) -> ScreenHandlerFuture<'_, ()> {
        Box::pin(async move {
            let slots_len = self.get_behaviour().slots.len();

            for i in 0..slots_len {
                let slot = self.get_behaviour().slots[i].clone();
                let stack = slot.get_cloned_stack().await;

                self.update_tracked_slot(i, stack.clone()).await;
                self.check_slot_updates(i, stack).await;
            }

            self.check_cursor_stack_updates().await;

            let behaviour = self.get_behaviour_mut();
            let mut prop_vec = vec![];
            for (idx, prop) in behaviour.properties.iter_mut().enumerate() {
                let value = prop.get();
                if prop.has_changed() {
                    prop_vec.push((idx, value));
                }
            }

            for (idx, value) in prop_vec {
                self.update_tracked_properties(idx as i32, value).await;
                self.check_property_updates(idx as i32, value).await;
            }
        })
    }

    /// Checks if a slot index is valid.
    fn is_slot_valid(&self, slot: i32) -> ScreenHandlerFuture<'_, bool> {
        Box::pin(async move { is_valid_slot_index(slot, self.get_behaviour().slots.len()) })
    }

    /// Disables synchronization (for batch operations).
    fn disable_sync(&mut self) {
        let behaviour = self.get_behaviour_mut();
        behaviour.disable_sync = true;
    }

    /// Re-enables synchronization.
    fn enable_sync(&mut self) {
        let behaviour = self.get_behaviour_mut();
        behaviour.disable_sync = false;
    }

    /// Gets the screen handler slot index for an inventory slot.
    fn get_slot_index<'a>(
        &'a self,
        inventory: &'a Arc<dyn Inventory>,
        slot: usize,
    ) -> OptionUsizeFuture<'a> {
        Box::pin(async move {
            (0..self.get_behaviour().slots.len()).find(|&i| {
                Arc::ptr_eq(&self.get_behaviour().slots[i].get_inventory(), inventory)
                    && self.get_behaviour().slots[i].get_index() == slot
            })
        })
    }

    /// Performs a quick move (shift-click) from a slot.
    ///
    /// Must be implemented by concrete screen handlers to define
    /// where items go when shift-clicked from specific slots.
    fn quick_move<'a>(
        &'a mut self,
        player: &'a dyn InventoryPlayer,
        slot_index: i32,
    ) -> ItemStackFuture<'a>;

    /// Handles a button click event (e.g., enchantment selection, beacon effects).
    fn on_button_click<'a>(
        &'a mut self,
        _player: &'a dyn InventoryPlayer,
        _button_id: i32,
    ) -> ScreenHandlerFuture<'a, bool> {
        Box::pin(async { false })
    }

    /// Inserts an item into a range of slots.
    ///
    /// First tries to stack with existing items, then fills empty slots.
    fn insert_item<'a>(
        &'a mut self,
        stack: &'a mut ItemStack,
        start_index: i32,
        end_index: i32,
        from_last: bool,
    ) -> ScreenHandlerFuture<'a, bool> {
        Box::pin(async move {
            let mut success = false;
            let mut current_index = if from_last {
                end_index - 1
            } else {
                start_index
            };

            if stack.is_stackable() {
                while !stack.is_empty()
                    && (if from_last {
                        current_index >= start_index
                    } else {
                        current_index < end_index
                    })
                {
                    let slot = self.get_behaviour().slots[current_index as usize].clone();
                    let mut slot_stack = slot.get_stack().await;

                    if !slot_stack.is_empty() && slot_stack.are_items_and_components_equal(stack) {
                        let max_slot_count = slot.get_max_item_count_for_stack(&slot_stack).await;
                        // Counts arrive from the network and may be malformed
                        // (or originate in a legacy save) even though normal
                        // vanilla stacks are much smaller than u8::MAX.  Do
                        // the arithmetic widened so a forged 255+255 stack
                        // cannot wrap and duplicate/delete items.
                        let combined_count =
                            u16::from(slot_stack.item_count) + u16::from(stack.item_count);
                        if combined_count <= u16::from(max_slot_count) {
                            stack.set_count(0);
                            slot_stack.set_count(combined_count as u8);
                            slot.set_stack(slot_stack).await;
                            success = true;
                        } else if slot_stack.item_count < max_slot_count {
                            stack.decrement(max_slot_count - slot_stack.item_count);
                            slot_stack.set_count(max_slot_count);
                            slot.set_stack(slot_stack).await;
                            success = true;
                        }
                    }

                    if from_last {
                        current_index -= 1;
                    } else {
                        current_index += 1;
                    }
                }
            }

            if !stack.is_empty() {
                if from_last {
                    current_index = end_index - 1;
                } else {
                    current_index = start_index;
                }

                while if from_last {
                    current_index >= start_index
                } else {
                    current_index < end_index
                } {
                    let slot = self.get_behaviour().slots[current_index as usize].clone();
                    let slot_stack = slot.get_stack().await;

                    if slot_stack.is_empty() && slot.can_insert(stack).await {
                        let max_count = slot.get_max_item_count_for_stack(stack).await;
                        slot.set_stack(stack.split(max_count.min(stack.item_count)))
                            .await;
                        slot.mark_dirty().await;
                        success = true;
                        break;
                    }

                    if from_last {
                        current_index -= 1;
                    } else {
                        current_index += 1;
                    }
                }
            }

            success
        })
    }

    /// Handles a slot click event.
    ///
    /// Override for custom click handling. Return true to prevent default handling.
    fn handle_slot_click<'a>(
        &'a self,
        _player: &'a dyn InventoryPlayer,
        _click_type: MouseClick,
        _slot: Arc<dyn Slot>,
        _slot_stack: ItemStack,
        _cursor_stack: ItemStack,
    ) -> ScreenHandlerFuture<'a, bool> {
        Box::pin(async {
            // TODO: required for bundle in the future
            false
        })
    }

    /// Cancels any client-side changes and resynchronizes the state.
    fn cancel(&mut self) -> ScreenHandlerFuture<'_, ()> {
        Box::pin(async move {
            self.get_behaviour_mut().reset_quick_craft();
            self.sync_state().await;
        })
    }

    /// Public entry point for slot click handling.
    fn on_slot_click<'a>(
        &'a mut self,
        slot_index: i32,
        button: i32,
        action_type: SlotActionType,
        player: &'a dyn InventoryPlayer,
    ) -> ScreenHandlerFuture<'a, ()> {
        Box::pin(async move {
            self.internal_on_slot_click(slot_index, button, action_type, player)
                .await;
        })
    }

    /// Internal slot click handling implementation.
    ///
    /// Handles all click types: pickup, quick move, swap, throw, drag, clone.
    #[expect(clippy::too_many_lines)]
    fn internal_on_slot_click<'a>(
        &'a mut self,
        slot_index: i32,
        button: i32,
        action_type: SlotActionType,
        player: &'a dyn InventoryPlayer,
    ) -> ScreenHandlerFuture<'a, ()> {
        Box::pin(async move {
            // A malicious or stale client may send a slot outside the current
            // menu after a close/reopen race.  Every branch below indexes the
            // slot vector, so reject it and resync instead of panicking the
            // server.  -999 is the vanilla outside-inventory sentinel and is
            // intentionally handled by the pickup/throw branches.
            let slot_count = self.get_behaviour().slots.len();
            if slot_index >= 0 && slot_index as usize >= slot_count {
                warn!("Ignoring out-of-range container slot {slot_index} (size {slot_count})");
                self.cancel().await;
                return;
            }

            if action_type != SlotActionType::QuickCraft && self.get_behaviour().drag_status != 0 {
                // AbstractContainerMenu resets an unfinished drag and ignores
                // the interleaved click.  Processing it as a normal pickup can
                // duplicate the carried stack after a forged packet sequence.
                self.get_behaviour_mut().reset_quick_craft();
                return;
            }

            if action_type == SlotActionType::PickupAll && (button == 0 || button == 1) {
                if slot_index < 0 {
                    return;
                }
                let behaviour = self.get_behaviour_mut();
                let mut cursor_stack = behaviour.cursor_stack.lock().await;
                if cursor_stack.is_empty() {
                    return;
                }

                let slot_count = behaviour.slots.len();
                let step = if button == 0 { 1 } else { -1 };
                let mut to_pick_up = cursor_stack
                    .get_max_stack_size()
                    .saturating_sub(cursor_stack.item_count);

                // Vanilla performs two passes: full stacks are deferred until
                // all partially-filled matching stacks have been collected.
                for pass in 0..2 {
                    let mut index = if button == 0 {
                        0
                    } else {
                        slot_count as i32 - 1
                    };
                    while index >= 0 && (index as usize) < slot_count && to_pick_up > 0 {
                        let slot = behaviour.slots[index as usize].clone();
                        let item_stack = slot.get_cloned_stack().await;
                        if item_stack.are_items_and_components_equal(&cursor_stack)
                            && (pass == 1
                                || item_stack.item_count < item_stack.get_max_stack_size())
                            && slot.can_take_items(player).await
                        {
                            let taken_stack = slot
                                .safe_take(
                                    item_stack.item_count.min(to_pick_up),
                                    cursor_stack
                                        .get_max_stack_size()
                                        .saturating_sub(cursor_stack.item_count),
                                    player,
                                )
                                .await;
                            to_pick_up = to_pick_up.saturating_sub(taken_stack.item_count);
                            cursor_stack.increment(taken_stack.item_count);
                        }
                        index += step;
                    }
                }
            } else if action_type == SlotActionType::QuickCraft {
                if button < 0 {
                    self.get_behaviour_mut().reset_quick_craft();
                    return;
                }
                let header = (button & 3) as u8;
                let quick_craft_type = ((button >> 2) & 3) as u8;
                let behaviour = self.get_behaviour_mut();
                let status = behaviour.drag_status;

                // The low two bits are a state machine (start/select/end),
                // not merely a hint.  Reject transitions that vanilla would
                // reset, including an invalid creative type.
                if (status != 1 || header != 2) && status != header {
                    behaviour.reset_quick_craft();
                    return;
                }
                if behaviour.cursor_stack.lock().await.is_empty() {
                    behaviour.reset_quick_craft();
                    return;
                }

                match header {
                    0 => {
                        if quick_craft_type > 1
                            && (quick_craft_type != 2 || !player.has_infinite_materials())
                        {
                            behaviour.reset_quick_craft();
                        } else {
                            behaviour.drag_type = quick_craft_type;
                            behaviour.drag_status = 1;
                            behaviour.drag_slots.clear();
                        }
                    }
                    1 => {
                        if slot_index < 0 || slot_index as usize >= behaviour.slots.len() {
                            behaviour.reset_quick_craft();
                            return;
                        }
                        let cursor_stack = behaviour.cursor_stack.lock().await;
                        let slot = behaviour.slots[slot_index as usize].clone();
                        let stack = slot.get_stack().await;
                        let can_add = !cursor_stack.is_empty()
                            && slot.can_insert(&cursor_stack).await
                            && (stack.are_items_and_components_equal(&cursor_stack)
                                || stack.is_empty())
                            && (behaviour.drag_type == 2
                                || usize::from(cursor_stack.item_count)
                                    > behaviour.drag_slots.len())
                            && !behaviour.drag_slots.contains(&(slot_index as u32));
                        if can_add {
                            // Full slots are admitted and skipped at commit;
                            // their presence still affects the vanilla
                            // per-slot division count.
                            behaviour.drag_slots.push(slot_index as u32);
                        }
                    }
                    2 => {
                        if behaviour.drag_slots.is_empty() {
                            behaviour.reset_quick_craft();
                            return;
                        }
                        let quick_type = behaviour.drag_type;
                        let selected = behaviour.drag_slots.clone();
                        if selected.len() == 1 {
                            let slot = selected[0] as i32;
                            behaviour.reset_quick_craft();
                            let _ = behaviour;
                            self.internal_on_slot_click(
                                slot,
                                quick_type as i32,
                                SlotActionType::Pickup,
                                player,
                            )
                            .await;
                            return;
                        }

                        let source_stack = behaviour.cursor_stack.lock().await.clone();
                        let mut slot_refs = Vec::with_capacity(selected.len());
                        let mut snapshots = Vec::with_capacity(selected.len());
                        for selected_slot in &selected {
                            let slot = behaviour.slots[*selected_slot as usize].clone();
                            let stack = slot.get_cloned_stack().await;
                            let eligible = slot.can_insert(&source_stack).await
                                && (stack.is_empty()
                                    || stack.are_items_and_components_equal(&source_stack));
                            let max_count = if eligible {
                                slot.get_max_item_count_for_stack(&source_stack).await
                            } else {
                                0
                            };
                            slot_refs.push((*selected_slot, slot));
                            snapshots.push((*selected_slot, stack, max_count));
                        }

                        let (placements, remaining) =
                            calculate_quick_craft_plan(&source_stack, &snapshots, quick_type);
                        let mut cursor_stack = behaviour.cursor_stack.lock().await;
                        // The cursor is the transaction source.  If a plugin or
                        // another packet changed it while slots were read,
                        // abort without writing any slot.
                        let cursor_unchanged = cursor_stack.item_count == source_stack.item_count
                            && cursor_stack.are_items_and_components_equal(&source_stack);
                        if !cursor_unchanged {
                            drop(cursor_stack);
                            behaviour.reset_quick_craft();
                            return;
                        }
                        for placement in placements {
                            debug_assert!(placement.inserted > 0);
                            if let Some((_, slot)) =
                                slot_refs.iter().find(|(slot, _)| *slot == placement.slot)
                            {
                                slot.set_stack(placement.stack).await;
                            }
                        }
                        if quick_type == 2 || remaining == 0 {
                            *cursor_stack = ItemStack::EMPTY.clone();
                        } else {
                            cursor_stack.item_count = remaining;
                        }
                        drop(cursor_stack);
                        behaviour.reset_quick_craft();
                    }
                    _ => {}
                }
            } else if action_type == SlotActionType::Throw {
                if slot_index >= 0 && self.get_behaviour().cursor_stack.lock().await.is_empty() {
                    let slot = self.get_behaviour().slots[slot_index as usize].clone();
                    let prev_stack = slot.get_cloned_stack().await;
                    if !prev_stack.is_empty() {
                        if button == 1 {
                            // Throw all
                            while slot
                                .get_cloned_stack()
                                .await
                                .are_items_and_components_equal(&prev_stack)
                            {
                                let drop_stack =
                                    slot.safe_take(prev_stack.item_count, u8::MAX, player).await;
                                player.drop_item(drop_stack, true).await;
                                // player.handleCreativeModeItemDrop(itemStack);
                            }
                        } else {
                            let drop_stack = slot.safe_take(1, u8::MAX, player).await;
                            if !drop_stack.is_empty() {
                                slot.on_take_item(player, &drop_stack).await;
                                player.drop_item(drop_stack, true).await;
                            }
                        }
                    }
                }
            } else if action_type == SlotActionType::Clone {
                if player.has_infinite_materials() && slot_index >= 0 {
                    let behaviour = self.get_behaviour_mut();
                    let mut cursor_stack = behaviour.cursor_stack.lock().await;
                    if !cursor_stack.is_empty() {
                        return;
                    }
                    let slot = behaviour.slots[slot_index as usize].clone();
                    let stack = slot.get_stack().await;
                    *cursor_stack = stack.copy_with_count(stack.get_max_stack_size());
                }
            } else if (action_type == SlotActionType::Pickup
                || action_type == SlotActionType::QuickMove)
                && (button == 0 || button == 1)
            {
                let click_type = if button == 0 {
                    MouseClick::Left
                } else {
                    MouseClick::Right
                };

                // Drop item if outside inventory
                if slot_index == SLOT_INDEX_OUTSIDE {
                    let mut cursor_stack = self.get_behaviour().cursor_stack.lock().await;
                    if !cursor_stack.is_empty() {
                        if click_type == MouseClick::Left {
                            player.drop_item(cursor_stack.clone(), true).await;
                            *cursor_stack = ItemStack::EMPTY.clone();
                        } else {
                            player.drop_item(cursor_stack.split(1), true).await;
                        }
                    }
                } else if action_type == SlotActionType::QuickMove {
                    if slot_index < 0 {
                        return;
                    }

                    let slot = self.get_behaviour().slots[slot_index as usize].clone();

                    if !slot.can_take_items(player).await {
                        return;
                    }

                    let mut moved_stack = self.quick_move(player, slot_index).await;

                    while !moved_stack.is_empty()
                        && ItemStack::are_items_and_components_equal(
                            &slot.get_cloned_stack().await,
                            &moved_stack,
                        )
                    {
                        moved_stack = self.quick_move(player, slot_index).await;
                    }
                } else {
                    // Pickup
                    if slot_index < 0 {
                        return;
                    }

                    let slot = self.get_behaviour().slots[slot_index as usize].clone();

                    if click_type == MouseClick::Left {
                        slot.on_click(player).await;
                    }

                    let slot_stack = slot.get_cloned_stack().await;
                    let mut cursor_stack = self.get_behaviour().cursor_stack.lock().await;

                    if click_type == MouseClick::Right {
                        let mut intercepted = false;

                        if !cursor_stack.is_empty() {
                            let mut inner_slot_stack = slot.get_stack().await;
                            if let Some(bundle) = inner_slot_stack.get_data_component_mut::<pumpkin_data::data_component_impl::BundleContentsImpl>()
                                && bundle.try_insert(&mut cursor_stack) {
                                    slot.set_stack(inner_slot_stack).await;
                                    intercepted = true;
                                }
                        }

                        if !intercepted && !slot_stack.is_empty()
                            && let Some(bundle) = cursor_stack.get_data_component_mut::<pumpkin_data::data_component_impl::BundleContentsImpl>() {
                                let mut inner_slot_stack = slot.get_stack().await;
                                if bundle.try_insert(&mut inner_slot_stack) {
                                    if inner_slot_stack.item_count == 0 {
                                        inner_slot_stack = ItemStack::EMPTY.clone();
                                    }
                                    slot.set_stack(inner_slot_stack).await;
                                    intercepted = true;
                                }
                            }

                        if !intercepted && cursor_stack.is_empty() {
                            let mut inner_slot_stack = slot.get_stack().await;
                            if let Some(bundle) = inner_slot_stack.get_data_component_mut::<pumpkin_data::data_component_impl::BundleContentsImpl>()
                                && let Some(extracted) = bundle.try_extract() {
                                    *cursor_stack = extracted;
                                    slot.set_stack(inner_slot_stack).await;
                                    intercepted = true;
                                }
                        }

                        if !intercepted && slot_stack.is_empty()
                            && let Some(bundle) = cursor_stack.get_data_component_mut::<pumpkin_data::data_component_impl::BundleContentsImpl>()
                                && let Some(extracted) = bundle.try_extract() {
                                    slot.set_stack(extracted).await;
                                    intercepted = true;
                                }

                        if intercepted {
                            if cursor_stack.item_count == 0 {
                                *cursor_stack = ItemStack::EMPTY.clone();
                            }
                            slot.mark_dirty().await;
                            return;
                        }
                    }

                    if self
                        .handle_slot_click(
                            player,
                            click_type.clone(),
                            slot.clone(),
                            slot_stack.clone(),
                            cursor_stack.clone(),
                        )
                        .await
                    {
                        return;
                    }

                    if slot_stack.is_empty() {
                        if !cursor_stack.is_empty() {
                            let transfer_count = if click_type == MouseClick::Left {
                                cursor_stack.item_count
                            } else {
                                1
                            };
                            *cursor_stack = slot
                                .insert_stack_count(cursor_stack.clone(), transfer_count)
                                .await;
                        }
                    } else if slot.can_take_items(player).await {
                        if cursor_stack.is_empty() {
                            let take_count = if click_type == MouseClick::Left {
                                slot_stack.item_count
                            } else {
                                slot_stack.item_count.div_ceil(2)
                            };
                            let taken =
                                slot.try_take_stack_range(take_count, u8::MAX, player).await;
                            if let Some(taken) = taken {
                                // Reverse order of operations, shouldn't affect anything
                                *cursor_stack = taken.clone();
                                slot.on_take_item(player, &taken).await;
                            }
                        } else if slot.can_insert(&cursor_stack).await {
                            if ItemStack::are_items_and_components_equal(&slot_stack, &cursor_stack)
                            {
                                let insert_count = if click_type == MouseClick::Left {
                                    cursor_stack.item_count
                                } else {
                                    1
                                };
                                *cursor_stack = slot
                                    .insert_stack_count(cursor_stack.clone(), insert_count)
                                    .await;
                            } else if cursor_stack.item_count
                                <= slot.get_max_item_count_for_stack(&cursor_stack).await
                            {
                                let old_cursor_stack = cursor_stack.clone();
                                *cursor_stack = slot_stack.clone();
                                slot.set_stack(old_cursor_stack).await;
                            }
                        } else if ItemStack::are_items_and_components_equal(
                            &slot_stack,
                            &cursor_stack,
                        ) {
                            let taken = slot
                                .try_take_stack_range(
                                    slot_stack.item_count,
                                    cursor_stack
                                        .get_max_stack_size()
                                        .saturating_sub(cursor_stack.item_count),
                                    player,
                                )
                                .await;

                            if let Some(taken) = taken {
                                cursor_stack.increment(taken.item_count);
                                slot.on_take_item(player, &taken).await;
                            }
                        }
                    }

                    // Armor and off-hand are entity equipment slots, not
                    // merely player-inventory slots. Emit the final stack
                    // after a merge, swap, or removal so the packet contains
                    // the resulting count/components rather than the
                    // pre-click cursor. This also fixes armor items whose
                    // Equippable component is absent or malformed: the menu
                    // slot, not the item payload, is authoritative.
                    if let Some(target_slot) = player_screen_equipment_slot(slot_index) {
                        let new_stack = slot.get_cloned_stack().await;
                        if !ItemStack::are_items_and_components_equal(&slot_stack, &new_stack)
                            || slot_stack.item_count != new_stack.item_count
                        {
                            player
                                .enqueue_equipment_change(&target_slot, &new_stack)
                                .await;
                        }
                    }

                    slot.mark_dirty().await;
                }
            } else if action_type == SlotActionType::Swap
                && ((0..9).contains(&button) || button == 40)
            {
                if slot_index < 0 {
                    return;
                }
                let mut button_stack = player.get_inventory().get_stack(button as usize).await;
                let source_slot = self.get_behaviour().slots[slot_index as usize].clone();
                let source_stack = source_slot.get_cloned_stack().await;

                // Pressing the number key for the slot that already owns the
                // selected hotbar stack is a no-op.  Treating it as a move
                // would otherwise clear and reinsert the same slot while
                // racing the client transaction revision.
                let player_inventory: Arc<dyn Inventory> = player.get_inventory();
                if Arc::ptr_eq(&source_slot.get_inventory(), &player_inventory)
                    && source_slot.get_index() == button as usize
                {
                    return;
                }

                if !button_stack.is_empty() || !source_stack.is_empty() {
                    if button_stack.is_empty() {
                        if source_slot.can_take_items(player).await {
                            player
                                .get_inventory()
                                .set_stack(button as usize, source_stack.clone())
                                .await;
                            source_slot.set_stack(ItemStack::EMPTY.clone()).await;
                            source_slot.on_take_item(player, &source_stack).await;
                        }
                    } else if source_stack.is_empty() && source_slot.can_insert(&button_stack).await
                    {
                        let max_count = source_slot
                            .get_max_item_count_for_stack(&button_stack)
                            .await;
                        if button_stack.item_count > max_count {
                            // Keep the remainder in the hotbar.  The previous
                            // implementation inserted only the first part but
                            // left the original hotbar stack untouched, which
                            // duplicated the excess items on every number-key
                            // swap.
                            let inserted = button_stack.split(max_count);
                            source_slot.set_stack(inserted).await;
                            player
                                .get_inventory()
                                .set_stack(button as usize, button_stack)
                                .await;
                        } else {
                            player
                                .get_inventory()
                                .set_stack(button as usize, ItemStack::EMPTY.clone())
                                .await;
                            source_slot.set_stack(button_stack).await;
                        }
                    } else if source_slot.can_take_items(player).await
                        && source_slot.can_insert(&button_stack).await
                        && button_stack.item_count
                            <= source_slot
                                .get_max_item_count_for_stack(&button_stack)
                                .await
                    {
                        // Both sides contain items: number-key swaps the two
                        // stacks atomically from the client's point of view.
                        player
                            .get_inventory()
                            .set_stack(button as usize, source_stack)
                            .await;
                        source_slot.set_stack(button_stack).await;
                    }
                }
            }
        })
    }
}

pub trait ScreenHandlerListener: Send + Sync {
    fn on_slot_update<'a>(
        &'a self,
        _screen_handler: &'a ScreenHandlerBehaviour,
        _slot: u8,
        _stack: ItemStack,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
    fn on_property_update<'a>(
        &'a self,
        _screen_handler: &'a ScreenHandlerBehaviour,
        _property: u8,
        _value: i32,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub type SharedScreenHandler = Arc<Mutex<dyn ScreenHandler>>;

pub trait ScreenHandlerFactory: Send + Sync {
    fn create_screen_handler<'a>(
        &'a self,
        sync_id: u8,
        player_inventory: &'a Arc<PlayerInventory>,
        player: &'a dyn InventoryPlayer,
    ) -> BoxFuture<'a, Option<SharedScreenHandler>>;
    fn get_display_name(&self) -> TextComponent;
}

pub struct ScreenHandlerBehaviour {
    /// Slots in this screen handler (includes both container and player slots).
    pub slots: Vec<Arc<dyn Slot>>,
    /// Sync ID for client-server matching (matches the window ID in protocol).
    pub sync_id: u8,
    /// Registered listeners for slot/property changes.
    pub listeners: Vec<Arc<dyn ScreenHandlerListener>>,
    /// Sync handler for sending updates to the client.
    pub sync_handler: Option<Arc<SyncHandler>>,
    /// Current tracked stacks for comparison with previous state.
    //TODO: Check if this is needed
    pub tracked_stacks: Vec<ItemStack>,
    /// The item currently held by the player's cursor (held item).
    pub cursor_stack: Arc<Mutex<ItemStack>>,
    /// Previous tracked stacks for detecting changes that need syncing.
    pub previous_tracked_stacks: Vec<TrackedStack>,
    /// Previous cursor stack for detecting cursor changes.
    pub previous_cursor_stack: TrackedStack,
    /// Revision counter for sync tracking (increments on each change).
    pub revision: AtomicU32,
    /// Whether sync is temporarily disabled (for batch operations).
    pub disable_sync: bool,
    /// Container properties (furnace progress, enchantment levels, etc.).
    pub properties: Vec<ScreenProperty>,
    /// Tracked property values for detecting changes.
    pub tracked_property_values: Vec<i32>,
    /// The window type for this container ( determines client UI).
    pub window_type: Option<WindowType>,
    /// Slots selected during a drag operation (for multi-slot distribution).
    pub drag_slots: Vec<u32>,
    /// Vanilla quick-craft protocol state: 0 idle, 1 selecting, 2 finishing.
    drag_status: u8,
    /// Quick-craft distribution type: 0 even, 1 one-per-slot, 2 creative fill.
    drag_type: u8,
    /// Whether players can grab items out of the inventory.
    pub allow_grab_items: bool,
    /// Whether players can put items into the inventory from their own.
    pub allow_put_items: bool,
    /// Number of slots that belong to the container (not the player inventory).
    pub container_slots: usize,
}

#[cfg(test)]
mod tests {
    use super::{calculate_quick_craft_plan, is_valid_slot_index, player_screen_equipment_slot};
    use pumpkin_data::data_component_impl::EquipmentSlot;
    use pumpkin_data::{item::Item, item_stack::ItemStack};

    #[test]
    fn player_screen_equipment_indices_match_vanilla_menu() {
        assert!(matches!(
            player_screen_equipment_slot(5),
            Some(EquipmentSlot::Head(_))
        ));
        assert!(matches!(
            player_screen_equipment_slot(6),
            Some(EquipmentSlot::Chest(_))
        ));
        assert!(matches!(
            player_screen_equipment_slot(7),
            Some(EquipmentSlot::Legs(_))
        ));
        assert!(matches!(
            player_screen_equipment_slot(8),
            Some(EquipmentSlot::Feet(_))
        ));
        assert!(matches!(
            player_screen_equipment_slot(45),
            Some(EquipmentSlot::OffHand(_))
        ));
        assert!(player_screen_equipment_slot(44).is_none());
    }

    #[test]
    fn slot_validation_accepts_only_vanilla_sentinels_and_real_slots() {
        assert!(is_valid_slot_index(-1, 10));
        assert!(is_valid_slot_index(-999, 10));
        assert!(is_valid_slot_index(0, 10));
        assert!(is_valid_slot_index(9, 10));
        assert!(!is_valid_slot_index(-2, 10));
        assert!(!is_valid_slot_index(10, 10));
    }

    #[test]
    fn quick_craft_left_preserves_remainder_and_divides_evenly() {
        let source = ItemStack::new(10, &Item::COBBLESTONE);
        let slots = vec![
            (0, ItemStack::EMPTY.clone(), 64),
            (1, ItemStack::EMPTY.clone(), 64),
            (2, ItemStack::EMPTY.clone(), 64),
        ];

        let (placements, remaining) = calculate_quick_craft_plan(&source, &slots, 0);
        assert_eq!(remaining, 1);
        assert_eq!(placements.len(), 3);
        assert!(placements.iter().all(|placement| placement.inserted == 3));
        assert!(
            placements
                .iter()
                .all(|placement| placement.stack.item_count == 3)
        );
    }

    #[test]
    fn quick_craft_right_respects_existing_stack_and_slot_max() {
        let source = ItemStack::new(3, &Item::COBBLESTONE);
        let slots = vec![
            (0, ItemStack::new(63, &Item::COBBLESTONE), 64),
            (1, ItemStack::new(64, &Item::COBBLESTONE), 64),
            (2, ItemStack::EMPTY.clone(), 64),
        ];

        let (placements, remaining) = calculate_quick_craft_plan(&source, &slots, 1);
        assert_eq!(remaining, 1);
        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].stack.item_count, 64);
        assert_eq!(placements[1].stack.item_count, 1);
    }

    #[test]
    fn quick_craft_creative_fills_each_slot_without_cursor_duplication() {
        let source = ItemStack::new(1, &Item::COBBLESTONE);
        let slots = vec![
            (0, ItemStack::EMPTY.clone(), 64),
            (1, ItemStack::new(60, &Item::COBBLESTONE), 64),
        ];

        let (placements, remaining) = calculate_quick_craft_plan(&source, &slots, 2);
        assert_eq!(remaining, 0);
        assert_eq!(placements[0].stack.item_count, 64);
        assert_eq!(placements[1].stack.item_count, 64);
    }

    #[test]
    fn quick_craft_never_overstacks_unstackable_items() {
        let source = ItemStack::new(1, &Item::IRON_SWORD);
        let slots = vec![
            (0, ItemStack::EMPTY.clone(), 64),
            (1, ItemStack::EMPTY.clone(), 64),
        ];

        let (placements, remaining) = calculate_quick_craft_plan(&source, &slots, 2);
        assert_eq!(remaining, 0);
        assert_eq!(placements.len(), 2);
        assert!(
            placements
                .iter()
                .all(|placement| placement.stack.item_count == 1)
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickType {
    Left,
    Right,
    ShiftLeft,
    ShiftRight,
    Middle,
    Drop,
    ControlDrop,
    DoubleClick,
    NumberKey(u8),
    Unknown,
}

impl ScreenHandlerBehaviour {
    #[must_use]
    pub fn new(sync_id: u8, window_type: Option<WindowType>) -> Self {
        Self {
            slots: Vec::new(),
            sync_id,
            listeners: Vec::new(),
            sync_handler: None,
            tracked_stacks: Vec::new(),
            cursor_stack: Arc::new(Mutex::new(ItemStack::EMPTY.clone())),
            previous_tracked_stacks: Vec::new(),
            previous_cursor_stack: TrackedStack::EMPTY,
            revision: AtomicU32::new(0),
            disable_sync: false,
            properties: Vec::new(),
            tracked_property_values: Vec::new(),
            window_type,
            drag_slots: Vec::new(),
            drag_status: 0,
            drag_type: 0,
            allow_grab_items: true,
            allow_put_items: true,
            container_slots: 0,
        }
    }

    pub fn next_revision(&self) -> u32 {
        self.revision.fetch_add(1, Ordering::Relaxed);
        self.revision.fetch_and(32767, Ordering::Relaxed) & 32767
    }

    fn reset_quick_craft(&mut self) {
        self.drag_status = 0;
        self.drag_type = 0;
        self.drag_slots.clear();
    }
}
