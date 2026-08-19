use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::AtomicI32,
    atomic::{AtomicBool, Ordering},
};

use crate::block::entities::PropertyDelegate;
use pumpkin_data::block_properties::BlockProperties;
use pumpkin_data::data_component_impl::DataComponentImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion_brewing::{ITEM_RECIPES, POTION_RECIPES};
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::inventory::{Inventory, sync_read_items_from_nbt, sync_write_items_to_nbt};
use tokio::sync::RwLock;

pub struct BrewingStandBlockEntity {
    pub position: BlockPos,
    pub items: RwLock<[ItemStack; Self::INVENTORY_SIZE]>,
    pub dirty: AtomicBool,
    pub brew_time: AtomicI32,
    pub fuel: AtomicI32,
    pub last_potion_count: StdMutex<Option<[bool; 3]>>,
    pub ingredient_item: StdMutex<Option<&'static pumpkin_data::item::Item>>,
}

impl BrewingStandBlockEntity {
    pub const INVENTORY_SIZE: usize = 5; // 3 potion slots + 1 ingredient + 1 fuel
    pub const ID: &'static str = "minecraft:brewing_stand";

    /// Vanilla `PotionBrewing#isIngredient`: an item is accepted in the
    /// ingredient slot iff it occurs in one of the registered item or potion
    /// recipe ingredient lists.  Keeping this derived from generated recipe
    /// data prevents the hopper and menu paths from drifting apart.
    #[must_use]
    fn is_brewing_ingredient(item: &'static Item) -> bool {
        ITEM_RECIPES.iter().any(|recipe| {
            recipe
                .ingredient()
                .iter()
                .any(|ingredient| ingredient.id == item.id)
        }) || POTION_RECIPES.iter().any(|recipe| {
            recipe
                .ingredient()
                .iter()
                .any(|ingredient| ingredient.id == item.id)
        })
    }

    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        use std::array::from_fn;
        Self {
            position,
            items: RwLock::new(from_fn(|_| ItemStack::EMPTY.clone())),
            dirty: AtomicBool::new(false),
            brew_time: AtomicI32::new(0),
            fuel: AtomicI32::new(0),
            last_potion_count: StdMutex::new(None),
            ingredient_item: StdMutex::new(None),
        }
    }

    /// Check if the current ingredient matches the stored ingredient
    fn ingredient_matches(&self, ingredient: &ItemStack) -> bool {
        self.ingredient_item
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some_and(|stored| !ingredient.is_empty() && ingredient.get_item().id == stored.id)
    }

    /// Check if any potion slot has a valid recipe with the ingredient
    async fn is_brewable(&self, ingredient: &ItemStack) -> bool {
        if ingredient.is_empty() {
            return false;
        }

        let ingredient_id = ingredient.get_item().id;

        // Check potion recipes (water bottle -> potions, potion upgrades, etc.)
        let items = self.items.read().await;
        for slot_idx in 0..3usize {
            let slot = &items[slot_idx];
            if slot.is_empty() {
                continue;
            }

            // Check item recipes first (potion -> splash potion, splash -> lingering)
            for recipe in &ITEM_RECIPES {
                if slot.get_item().id == recipe.from().id
                    && recipe.ingredient().iter().any(|i| i.id == ingredient_id)
                {
                    return true;
                }
            }

            // Check potion recipes (modify potion type)
            if let Some(pc) =
                slot.get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
                && let Some(potion_id) = pc.potion_id
            {
                for recipe in &POTION_RECIPES {
                    if recipe.from().id as i32 == potion_id
                        && recipe.ingredient().iter().any(|i| i.id == ingredient_id)
                    {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Perform brewing on all valid potion slots
    async fn do_brew(&self, world: &Arc<crate::world::World>, ingredient: &ItemStack) {
        let ingredient_id = ingredient.get_item().id;

        // Apply recipes to each slot
        for slot_idx in 0..3usize {
            let items = self.items.read().await;
            let slot = &items[slot_idx];
            if slot.is_empty() {
                continue;
            }

            let mut new_stack_opt: Option<ItemStack> = None;

            // Try item recipes first (potion -> splash/lingering)
            for recipe in &ITEM_RECIPES {
                if slot.get_item().id == recipe.from().id
                    && recipe.ingredient().iter().any(|i| i.id == ingredient_id)
                {
                    let new_item = recipe.to();
                    let potion_comp = slot.get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>().cloned();
                    let new_stack = potion_comp.map_or_else(
                        || ItemStack::new(slot.item_count, new_item),
                        |pc| {
                            ItemStack::new_with_component(
                                slot.item_count,
                                new_item,
                                vec![(
                                    pumpkin_data::data_component::DataComponent::PotionContents,
                                    Some(pc.to_dyn()),
                                )],
                            )
                        },
                    );
                    new_stack_opt = Some(new_stack);
                    break;
                }
            }

            // Try potion recipes (modify potion type) if item recipe didn't apply
            if new_stack_opt.is_none()
                && let Some(pc) = slot
                    .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
                && let Some(potion_id) = pc.potion_id
            {
                for recipe in &POTION_RECIPES {
                    if recipe.from().id as i32 == potion_id
                        && recipe.ingredient().iter().any(|i| i.id == ingredient_id)
                    {
                        let new_pc = pumpkin_data::data_component_impl::PotionContentsImpl {
                            potion_id: Some(recipe.to().id as i32),
                            custom_color: pc.custom_color,
                            custom_effects: pc.custom_effects.clone(),
                            custom_name: pc.custom_name.clone(),
                        };
                        let new_stack = ItemStack::new_with_component(
                            slot.item_count,
                            slot.get_item(),
                            vec![(
                                pumpkin_data::data_component::DataComponent::PotionContents,
                                Some(new_pc.to_dyn()),
                            )],
                        );
                        new_stack_opt = Some(new_stack);
                        break;
                    }
                }
            }

            drop(items);

            // Update the slot using set_stack if a recipe was applied
            if let Some(new_stack) = new_stack_opt {
                self.set_stack(slot_idx, new_stack).await;
            }
        }

        // Consume ingredient
        let mut items = self.items.write().await;
        items[3].decrement(1);
        self.mark_dirty();
        drop(items);

        // Play sound at the center of the block
        let pos = Vector3::new(
            self.position.0.x as f64 + 0.5,
            self.position.0.y as f64 + 0.5,
            self.position.0.z as f64 + 0.5,
        );
        world.play_sound(Sound::BlockBrewingStandBrew, SoundCategory::Blocks, &pos);

        // Mark dirty to trigger update
        self.mark_dirty();

        // Brewing ingredients use the same recipe-remainder table as furnace
        // fuel/crafting ingredients.  Put the remainder back into the
        // ingredient slot only when the consumed stack became empty; if a
        // stack remains, vanilla keeps the remainder implicit and consumes it
        // on the next use instead of creating an extra item.
        if let Some(remainder_id) =
            pumpkin_data::recipe_remainder::get_recipe_remainder_id(ingredient.item.id)
            && let Some(remainder_item) = pumpkin_data::item::Item::from_id(remainder_id)
        {
            let mut items = self.items.write().await;
            if items[3].is_empty() {
                items[3] = ItemStack::new(1, remainder_item);
                self.mark_dirty();
            }
        }
    }
}

impl pumpkin_world::inventory::Inventory for BrewingStandBlockEntity {
    fn size(&self) -> usize {
        Self::INVENTORY_SIZE
    }

    fn is_empty(&self) -> pumpkin_world::inventory::InventoryFuture<'_, bool> {
        Box::pin(async move {
            let items = self.items.read().await;
            for slot in items.iter() {
                if !slot.is_empty() {
                    return false;
                }
            }
            true
        })
    }

    fn get_stack(&self, slot: usize) -> pumpkin_world::inventory::InventoryFuture<'_, ItemStack> {
        Box::pin(async move {
            let items = self.items.read().await;
            items[slot].clone()
        })
    }

    fn remove_stack(
        &self,
        slot: usize,
    ) -> pumpkin_world::inventory::InventoryFuture<'_, ItemStack> {
        Box::pin(async move {
            let mut items = self.items.write().await;
            let removed = std::mem::replace(&mut items[slot], ItemStack::EMPTY.clone());
            self.mark_dirty();
            removed
        })
    }

    fn remove_stack_specific(
        &self,
        slot: usize,
        amount: u8,
    ) -> pumpkin_world::inventory::InventoryFuture<'_, ItemStack> {
        Box::pin(async move {
            let mut items = self.items.write().await;
            let taken = if items[slot].item_count <= amount {
                std::mem::replace(&mut items[slot], ItemStack::EMPTY.clone())
            } else {
                let mut taken = items[slot].clone();
                taken.item_count = amount;
                items[slot].item_count -= amount;
                taken
            };
            self.mark_dirty();
            taken
        })
    }

    fn set_stack(
        &self,
        slot: usize,
        stack: ItemStack,
    ) -> pumpkin_world::inventory::InventoryFuture<'_, ()> {
        Box::pin(async move {
            let mut items = self.items.write().await;
            items[slot] = stack;
            self.mark_dirty();
        })
    }

    fn on_open(&self) -> pumpkin_world::inventory::InventoryFuture<'_, ()> {
        Box::pin(async move {})
    }

    fn on_close(&self) -> pumpkin_world::inventory::InventoryFuture<'_, ()> {
        Box::pin(async move {})
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn is_valid_slot_for(&self, slot: usize, stack: &ItemStack) -> bool {
        if stack.is_empty() {
            return true;
        }

        match slot {
            // Slots 0-2 - potion bottles
            0..=2 => {
                stack
                    .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
                    .is_some()
                    || matches!(
                        stack.get_item().id,
                        id if id == pumpkin_data::item::Item::GLASS_BOTTLE.id
                            || id == pumpkin_data::item::Item::POTION.id
                            || id == pumpkin_data::item::Item::SPLASH_POTION.id
                            || id == pumpkin_data::item::Item::LINGERING_POTION.id
                    )
            }
            // Slot 3 - ingredient (must be tagged as brewable)
            3 => Self::is_brewing_ingredient(stack.get_item()),
            // Slot 4 - fuel
            4 => stack.get_item().has_tag(&tag::Item::MINECRAFT_BREWING_FUEL),
            _ => false,
        }
    }

    fn can_insert_from_hopper(
        &self,
        slot: usize,
        stack: &ItemStack,
        direction: pumpkin_data::BlockDirection,
    ) -> bool {
        if stack.is_empty() {
            return true;
        }
        let face_allows_slot = match direction {
            // Vanilla SLOTS_FOR_UP = { INGREDIENT_SLOT }.
            pumpkin_data::BlockDirection::Up => slot == 3,
            // Vanilla SLOTS_FOR_DOWN = { 0, 1, 2, 3 }.
            pumpkin_data::BlockDirection::Down => slot <= 3,
            // Vanilla SLOTS_FOR_SIDES = { 0, 1, 2, FUEL_SLOT }.
            pumpkin_data::BlockDirection::North
            | pumpkin_data::BlockDirection::South
            | pumpkin_data::BlockDirection::West
            | pumpkin_data::BlockDirection::East => (slot <= 2) || slot == 4,
        };
        if !face_allows_slot || !self.is_valid_slot_for(slot, stack) {
            return false;
        }

        true
    }

    fn can_extract_to_hopper(
        &self,
        _hopper_inventory: &dyn pumpkin_world::inventory::Inventory,
        slot: usize,
        stack: &ItemStack,
        direction: pumpkin_data::BlockDirection,
    ) -> bool {
        let face_allows_slot = match direction {
            // A hopper pulling from this stand's top sees DOWN.  Vanilla's
            // bottom slot list excludes fuel and permits bottle outputs plus
            // the ingredient slot when it contains a glass bottle.
            pumpkin_data::BlockDirection::Down => slot <= 3,
            pumpkin_data::BlockDirection::Up => slot == 3,
            pumpkin_data::BlockDirection::North
            | pumpkin_data::BlockDirection::South
            | pumpkin_data::BlockDirection::West
            | pumpkin_data::BlockDirection::East => (slot <= 2) || slot == 4,
        };
        face_allows_slot
            && (slot != 3 || stack.get_item().id == pumpkin_data::item::Item::GLASS_BOTTLE.id)
    }

    fn can_merge_from_hopper(
        &self,
        slot: usize,
        _current: &ItemStack,
        _incoming: &ItemStack,
        _direction: pumpkin_data::BlockDirection,
    ) -> bool {
        // Potion/bottle slots must be empty; fuel and the ingredient slot may
        // receive additional matching items.
        slot >= 3
    }
}

impl pumpkin_world::inventory::Clearable for BrewingStandBlockEntity {
    fn clear(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            let mut items = self.items.write().await;
            items.fill_with(|| ItemStack::EMPTY.clone());
            self.mark_dirty();
        })
    }
}

impl crate::block::entities::BlockEntity for BrewingStandBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let mut entity = Self::new(position);

        // Load brew time / fuel if present in NBT
        if let Some(bt) = nbt.get_int("BrewTime") {
            entity.brew_time.store(bt, Ordering::Relaxed);
        }
        if let Some(f) = nbt.get_int("Fuel") {
            entity.fuel.store(f, Ordering::Relaxed);
        }

        // Load inventory items from NBT
        let items = entity.items.get_mut();
        sync_read_items_from_nbt(nbt, items);

        // If there's an ingredient in slot 3, remember its base item for matching
        let ingredient_item = (!items[3].is_empty()).then(|| items[3].get_item());

        // Recompute last_potion_count so visuals are correct after load
        let mut current: [bool; 3] = [false; 3];
        for (i, slot) in items.iter().take(3).enumerate() {
            current[i] = !slot.is_empty()
                && (slot
                    .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
                    .is_some()
                    || slot.get_item().id == pumpkin_data::item::Item::GLASS_BOTTLE.id);
        }

        if let Some(item) = ingredient_item {
            *entity
                .ingredient_item
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(item);
        }

        *entity
            .last_potion_count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(current);

        entity
    }

    fn write_nbt<'a>(
        &'a self,
        nbt: &'a mut NbtCompound,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            // Persist brew state
            nbt.put_int("BrewTime", self.brew_time.load(Ordering::Relaxed));
            nbt.put_int("Fuel", self.fuel.load(Ordering::Relaxed));

            // Save inventory contents to NBT
            self.write_inventory_nbt(nbt, true).await;
        })
    }

    fn get_inventory(self: Arc<Self>) -> Option<Arc<dyn Inventory>> {
        Some(self)
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        nbt.put_int("BrewTime", self.brew_time.load(Ordering::Relaxed));
        nbt.put_int("Fuel", self.fuel.load(Ordering::Relaxed));
        sync_write_items_to_nbt(&*futures::executor::block_on(self.items.read()), &mut nbt);
        Some(nbt)
    }

    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    fn clear_dirty(&self) {
        self.dirty.store(false, Ordering::Relaxed);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn tick<'a>(
        &'a self,
        world: &'a Arc<crate::world::World>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            // Refill fuel counter from fuel item if needed
            let fuel_refilled = if self.fuel.load(Ordering::Relaxed) <= 0 {
                let mut items = self.items.write().await;
                if !items[4].is_empty()
                    && items[4]
                        .get_item()
                        .has_tag(&tag::Item::MINECRAFT_BREWING_FUEL)
                {
                    self.fuel.store(20, Ordering::Relaxed);
                    items[4].decrement(1);
                    true
                } else {
                    false
                }
            } else {
                false
            };

            // Get current ingredient and check brewing state
            let ingredient = self.items.read().await[3].clone();
            let brewable = self.is_brewable(&ingredient).await;
            let is_brewing = self.brew_time.load(Ordering::Relaxed) > 0;

            // Handle brewing state machine
            if is_brewing {
                // Decrement brew time
                let new_brew_time = self.brew_time.fetch_sub(1, Ordering::Relaxed) - 1;
                let is_done_brewing = new_brew_time == 0;

                if is_done_brewing && brewable {
                    // Brewing complete
                    self.do_brew(world, &ingredient).await;
                } else if !brewable || !self.ingredient_matches(&ingredient) {
                    // Cancel brewing
                    self.brew_time.store(0, Ordering::Relaxed);
                    self.mark_dirty();
                } else {
                    // Continue brewing
                    self.mark_dirty();
                }
            } else if brewable && self.fuel.load(Ordering::Relaxed) > 0 {
                // Start new brewing cycle
                self.fuel.fetch_sub(1, Ordering::Relaxed);
                self.brew_time.store(400, Ordering::Relaxed);
                *self
                    .ingredient_item
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(ingredient.get_item());
                self.mark_dirty();
            } else if fuel_refilled {
                // Mark dirty if fuel was refilled to update fuel indicator
                self.mark_dirty();
            }

            // Ensure clients are notified when potion slot contents (and their data) change.
            // Compute current presence bits for the three bottle slots
            let mut current: [bool; 3] = [false; 3];
            let items_guard = self.items.read().await;
            for (i, slot) in items_guard.iter().take(3).enumerate() {
                // Consider a potion slot "present" when it has an item and a PotionContents component or is a glass bottle
                current[i] = !slot.is_empty() && (slot.get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>().is_some() || slot.get_item().id == Item::GLASS_BOTTLE.id);
            }
            drop(items_guard);

            // If potion presence changed, update last_potion_count and update block state so clients
            let mut needs_update = false;
            {
                let mut last_guard = self
                    .last_potion_count
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if last_guard.as_ref() != Some(&current) {
                    *last_guard = Some(current);
                    needs_update = true;
                }
            }

            if needs_update {
                // Update the block state properties for the brewing stand to reflect bottle presence
                let world = world.clone();
                let (block, state) = world.get_block_and_state(&self.position);
                // Use generated block properties helper to produce a new state id with the bits set
                let mut props =
                    pumpkin_data::block_properties::BrewingStandLikeProperties::from_state_id(
                        state.id, block,
                    );
                // Generated field names use raw identifiers for clarity
                props.r#has_bottle_0 = current[0];
                props.r#has_bottle_1 = current[1];
                props.r#has_bottle_2 = current[2];

                world
                    .set_block_state(
                        &self.position,
                        props.to_state_id(block),
                        crate::world::BlockFlags::NOTIFY_ALL,
                    )
                    .await;

                // Also mark dirty so inventory/container updates are sent to open screens
                self.mark_dirty();
            }
        })
    }

    fn to_property_delegate(self: Arc<Self>) -> Option<Arc<dyn PropertyDelegate>> {
        Some(self as Arc<dyn PropertyDelegate>)
    }
}

impl PropertyDelegate for BrewingStandBlockEntity {
    fn get_property(&self, index: i32) -> i32 {
        match index {
            0 => self.brew_time.load(Ordering::Relaxed),
            1 => self.fuel.load(Ordering::Relaxed),
            _ => 0,
        }
    }

    fn set_property(&self, _index: i32, _value: i32) {}

    fn get_properties_size(&self) -> i32 {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::BrewingStandBlockEntity;
    use pumpkin_data::item::Item;

    #[test]
    fn brewing_ingredient_comes_from_generated_recipe_tables() {
        assert!(BrewingStandBlockEntity::is_brewing_ingredient(
            &Item::NETHER_WART
        ));
        assert!(BrewingStandBlockEntity::is_brewing_ingredient(
            &Item::GUNPOWDER
        ));
        assert!(!BrewingStandBlockEntity::is_brewing_ingredient(&Item::DIRT));
    }
}
