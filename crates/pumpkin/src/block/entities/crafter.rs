use crate::block::entities::BlockEntity;
use pumpkin_data::data_component_impl::UseRemainderImpl;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::{Block, block_properties::BlockProperties};
use pumpkin_inventory::crafting::crafting_screen_handler::find_crafting_result;
use pumpkin_inventory::crafting::recipe_provider::RecipeProvider;
use pumpkin_inventory::crafting::recipes::RecipeInputInventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::inventory::{Clearable, Inventory, InventoryFuture, sync_write_items_to_nbt};
use pumpkin_world::world::BlockFlags;
use std::any::Any;
use std::array::from_fn;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

pub struct CrafterBlockEntity {
    pub position: BlockPos,
    pub items: tokio::sync::RwLock<[ItemStack; Self::INVENTORY_SIZE]>,
    pub crafting_ticks_remaining: AtomicI32,
    pub disabled_slots: std::sync::atomic::AtomicU16,
    pub triggered: AtomicBool,
    pub dirty: AtomicBool,
}

impl BlockEntity for CrafterBlockEntity {
    fn write_nbt<'a>(
        &'a self,
        nbt: &'a mut NbtCompound,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let items = self.items.read().await;
            sync_write_items_to_nbt(items.as_slice(), nbt);
            nbt.put_int(
                "crafting_ticks_remaining",
                self.crafting_ticks_remaining.load(Ordering::Relaxed),
            );
            nbt.put_int_array(
                "disabled_slots",
                Self::disabled_slot_indices(self.disabled_slots.load(Ordering::Relaxed)),
            );
            nbt.put_bool("triggered", self.triggered.load(Ordering::Relaxed));
        })
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let mut items = from_fn(|_| ItemStack::EMPTY.clone());
        pumpkin_world::inventory::sync_read_items_from_nbt(nbt, &mut items);
        let crafter = Self {
            position,
            items: tokio::sync::RwLock::new(items),
            crafting_ticks_remaining: AtomicI32::new(
                nbt.get_int("crafting_ticks_remaining").unwrap_or(0),
            ),
            disabled_slots: std::sync::atomic::AtomicU16::new(Self::read_disabled_slots(nbt)),
            triggered: AtomicBool::new(nbt.get_bool("triggered").unwrap_or(false)),
            dirty: AtomicBool::new(false),
        };

        crafter
    }

    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn get_inventory(self: Arc<Self>) -> Option<Arc<dyn Inventory>> {
        Some(self)
    }

    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    fn clear_dirty(&self) {
        self.dirty.store(false, Ordering::Relaxed);
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        let items = futures::executor::block_on(self.items.read());
        sync_write_items_to_nbt(items.as_slice(), &mut nbt);
        nbt.put_int(
            "crafting_ticks_remaining",
            self.crafting_ticks_remaining.load(Ordering::Relaxed),
        );
        nbt.put_int_array(
            "disabled_slots",
            Self::disabled_slot_indices(self.disabled_slots.load(Ordering::Relaxed)),
        );
        nbt.put_bool("triggered", self.triggered.load(Ordering::Relaxed));
        Some(nbt)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn tick<'a>(
        &'a self,
        world: &'a std::sync::Arc<crate::world::World>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let remaining = self.crafting_ticks_remaining.load(Ordering::Relaxed);
            if remaining <= 0 {
                return;
            }
            let next = self
                .crafting_ticks_remaining
                .fetch_sub(1, Ordering::Relaxed)
                - 1;
            if next == 0 {
                let block = world.get_block(&self.position);
                let state = world.get_block_state(&self.position);
                if block.id == Block::CRAFTER.id {
                    let mut props =
                        pumpkin_data::block_properties::CrafterLikeProperties::from_state_id(
                            state.id, block,
                        );
                    if props.crafting {
                        props.crafting = false;
                        world
                            .set_block_state(
                                &self.position,
                                props.to_state_id(block),
                                BlockFlags::NOTIFY_LISTENERS,
                            )
                            .await;
                        world
                            .update_comparators(&self.position, &Block::CRAFTER)
                            .await;
                        self.mark_dirty();
                    }
                }
            }
        })
    }
}

impl CrafterBlockEntity {
    pub const INVENTORY_SIZE: usize = 9;
    pub const ID: &'static str = "minecraft:crafter";

    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            items: tokio::sync::RwLock::new(from_fn(|_| ItemStack::EMPTY.clone())),
            crafting_ticks_remaining: AtomicI32::new(0),
            disabled_slots: std::sync::atomic::AtomicU16::new(0),
            triggered: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
        }
    }

    /// Matches and executes one crafter operation.
    ///
    /// The returned list is ordered exactly like vanilla's
    /// `CrafterBlock.dispenseFrom`: the crafted result first, followed by one
    /// recipe remainder for each consumed ingredient.  The block places every
    /// stack in its front container (or drops it when that container cannot
    /// accept it).  Keeping the remainders in this transaction, rather than
    /// silently putting them back into their source slots, is important for
    /// milk buckets, honey bottles, decorated-pot ingredients, and any future
    /// component-defined `use_remainder` item.
    pub async fn craft_once(&self, provider: &dyn RecipeProvider) -> Option<Vec<ItemStack>> {
        // Recipe matching is asynchronous because the shared matcher also
        // serves player menus.  Capture the complete input transaction first,
        // then validate that snapshot while holding the write lock before
        // consuming anything.  A player/hopper update racing this method now
        // causes a failed pulse instead of consuming a different recipe's
        // ingredients or deleting an input.
        let input_snapshot = {
            let items = self.items.read().await;
            items.clone()
        };
        let disabled_slots = self.disabled_slots.load(Ordering::Acquire);
        let result = find_crafting_result(self, Some(provider)).await?;
        let item = pumpkin_data::item::Item::from_registry_key(
            result
                .item_id
                .strip_prefix("minecraft:")
                .unwrap_or(&result.item_id),
        )?;

        // Limited crafting is a player rule; automated Crafters are not
        // associated with a player and therefore always use the authoritative
        // recipe registry.  The stable recipe key is retained in `RecipeResult`
        // so callers can award advancements/statistics without guessing from
        // the output item.

        let mut items = self.items.write().await;
        if self.disabled_slots.load(Ordering::Acquire) != disabled_slots
            || items
                .iter()
                .zip(input_snapshot.iter())
                .any(|(current, snapshot)| !Self::same_stack(current, snapshot))
        {
            return None;
        }

        // Crafter recipes consume one item from every occupied input slot.
        // The component-based UseRemainder template is authoritative; the
        // generated legacy table remains a compatibility fallback for older
        // item data.  Vanilla sends each remainder through the same output
        // path as the recipe result, so collect them before mutating inputs.
        let mut outputs = Vec::with_capacity(Self::INVENTORY_SIZE + 1);
        outputs.push(Self::result_stack(&result, &item));
        for slot in 0..Self::INVENTORY_SIZE {
            if disabled_slots & (1u16 << slot) != 0 || items[slot].is_empty() {
                continue;
            }
            let remainder = Self::recipe_remainder_for(&items[slot]);
            items[slot].decrement(1);
            if let Some(remainder) = remainder {
                outputs.push(remainder);
            }
        }
        drop(items);
        self.mark_dirty();

        Some(outputs)
    }

    fn result_stack(
        result: &pumpkin_inventory::crafting::crafting_screen_handler::RecipeResult,
        item: &'static pumpkin_data::item::Item,
    ) -> ItemStack {
        // `RecipeResult::components` is a component patch, not a complete
        // item definition.  Constructing the stack through the canonical NBT
        // reader applies that patch over the generated item defaults and keeps
        // special recipes (notably decorated pots) lossless in Crafter output.
        let mut item_nbt = NbtCompound::new();
        item_nbt.put_string("id", format!("minecraft:{}", item.registry_key));
        item_nbt.put_int("count", i32::from(result.count));
        if let Some(components) = &result.components {
            item_nbt.put_compound("components", components.clone());
        }
        ItemStack::read_item_stack(&item_nbt).unwrap_or_else(|| ItemStack::new(result.count, item))
    }

    pub fn set_crafting_ticks_remaining(&self, ticks: i32) {
        self.crafting_ticks_remaining
            .store(ticks.max(0), Ordering::Relaxed);
    }

    pub fn set_triggered(&self, triggered: bool) {
        self.triggered.store(triggered, Ordering::Release);
        self.mark_dirty();
    }

    /// Vanilla only allows an empty slot to be disabled.  Setting an item into
    /// that slot re-enables it; the menu and hopper paths both call through
    /// this mutation boundary.
    pub fn set_slot_disabled(&self, slot: usize, disabled: bool) {
        if slot >= Self::INVENTORY_SIZE {
            return;
        }
        let Ok(items) = self.items.try_read() else {
            return;
        };
        if disabled && !items[slot].is_empty() {
            return;
        }
        drop(items);
        let mask = 1u16 << slot;
        if disabled {
            self.disabled_slots.fetch_or(mask, Ordering::AcqRel);
        } else {
            self.disabled_slots.fetch_and(!mask, Ordering::AcqRel);
        }
        self.mark_dirty();
    }

    fn disabled_slot_indices(mask: u16) -> Vec<i32> {
        (0..Self::INVENTORY_SIZE)
            .filter(|slot| mask & (1u16 << slot) != 0)
            .map(|slot| slot as i32)
            .collect()
    }

    fn read_disabled_slots(nbt: &NbtCompound) -> u16 {
        if let Some(slots) = nbt.get_int_array("disabled_slots") {
            return slots.iter().fold(0u16, |mask, slot| {
                if (0..Self::INVENTORY_SIZE as i32).contains(slot) {
                    mask | (1u16 << (*slot as usize))
                } else {
                    mask
                }
            });
        }
        // Accept the old local scalar form so existing Pumpkin worlds upgrade
        // without losing disabled-slot state, but always write canonical 26.2
        // int-array NBT from this point onward.
        nbt.get_int("disabled_slots")
            .unwrap_or_default()
            .clamp(0, i32::from(u16::MAX)) as u16
    }

    #[must_use]
    pub fn recipe_remainder(item_id: u16) -> Option<&'static pumpkin_data::item::Item> {
        pumpkin_data::recipe_remainder::get_recipe_remainder_id(item_id)
            .and_then(pumpkin_data::item::Item::from_id)
    }

    fn recipe_remainder_for(stack: &ItemStack) -> Option<ItemStack> {
        stack
            .get_data_component::<UseRemainderImpl>()
            .map(|remainder| ItemStack::new(remainder.count, remainder.remainder))
            .or_else(|| Self::recipe_remainder(stack.item.id).map(|item| ItemStack::new(1, item)))
    }

    fn same_stack(left: &ItemStack, right: &ItemStack) -> bool {
        left.item_count == right.item_count && left.are_items_and_components_equal(right)
    }
}

impl Inventory for CrafterBlockEntity {
    fn size(&self) -> usize {
        Self::INVENTORY_SIZE
    }

    fn is_empty(&self) -> InventoryFuture<'_, bool> {
        Box::pin(async move {
            let items = self.items.read().await;
            items.iter().all(ItemStack::is_empty)
        })
    }

    fn get_stack(&self, slot: usize) -> InventoryFuture<'_, ItemStack> {
        Box::pin(async move {
            let items = self.items.read().await;
            items[slot].clone()
        })
    }

    fn remove_stack(&self, slot: usize) -> InventoryFuture<'_, ItemStack> {
        Box::pin(async move {
            let mut items = self.items.write().await;
            let removed = std::mem::replace(&mut items[slot], ItemStack::EMPTY.clone());
            self.mark_dirty();
            removed
        })
    }

    fn remove_stack_specific(&self, slot: usize, amount: u8) -> InventoryFuture<'_, ItemStack> {
        Box::pin(async move {
            let mut items = self.items.write().await;
            let res = if !items[slot].is_empty() && amount > 0 {
                items[slot].split(amount)
            } else {
                ItemStack::EMPTY.clone()
            };
            self.mark_dirty();
            res
        })
    }

    fn set_stack(&self, slot: usize, stack: ItemStack) -> InventoryFuture<'_, ()> {
        Box::pin(async move {
            let mut items = self.items.write().await;
            if slot < Self::INVENTORY_SIZE && !stack.is_empty() {
                self.disabled_slots
                    .fetch_and(!(1u16 << slot), Ordering::AcqRel);
            }
            items[slot] = stack;
            self.mark_dirty();
        })
    }

    fn is_valid_slot_for(&self, slot: usize, stack: &ItemStack) -> bool {
        if slot >= Self::INVENTORY_SIZE || stack.is_empty() {
            return false;
        }
        let Ok(items) = self.items.try_read() else {
            // A hopper must never bypass a disabled/full slot while the
            // crafter is being edited.  A conservative rejection retries on
            // the next hopper tick and cannot duplicate or overwrite input.
            return false;
        };
        if self.disabled_slots.load(Ordering::Acquire) & (1u16 << slot) != 0 {
            return false;
        }
        let current = &items[slot];
        if current.item_count >= current.get_max_stack_size() {
            return false;
        }
        if current.is_empty() || !current.are_items_and_components_equal(stack) {
            return true;
        }

        // CrafterSlot.canPlaceItem delegates to the vanilla smaller-stack
        // ordering rule: when equal items are already present in multiple
        // slots, a hopper fills the lowest-count slot first and does not
        // starve an empty/lower slot later in the row.
        for later in (slot + 1)..Self::INVENTORY_SIZE {
            if self.disabled_slots.load(Ordering::Acquire) & (1u16 << later) != 0 {
                continue;
            }
            let later_stack = &items[later];
            if later_stack.is_empty()
                || (later_stack.are_items_and_components_equal(current)
                    && later_stack.item_count < current.item_count)
            {
                return false;
            }
        }
        true
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl RecipeInputInventory for CrafterBlockEntity {
    fn get_width(&self) -> usize {
        3
    }

    fn get_height(&self) -> usize {
        3
    }

    fn is_slot_enabled(&self, slot: usize) -> bool {
        slot < Self::INVENTORY_SIZE
            && self.disabled_slots.load(Ordering::Relaxed) & (1u16 << slot) == 0
    }
}

impl Clearable for CrafterBlockEntity {
    fn clear(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            let mut items = self.items.write().await;
            items.fill_with(|| ItemStack::EMPTY.clone());
            self.mark_dirty();
        })
    }
}

#[cfg(test)]
mod tests {
    use super::CrafterBlockEntity;
    use crate::block::entities::BlockEntity;
    use pumpkin_data::item::Item;
    use pumpkin_data::item_stack::ItemStack;
    use pumpkin_inventory::crafting::recipes::RecipeInputInventory;
    use pumpkin_nbt::compound::NbtCompound;
    use pumpkin_util::math::position::BlockPos;
    use pumpkin_world::inventory::Inventory;

    #[tokio::test]
    async fn crafter_inventory_and_disabled_slots_round_trip() {
        let source = CrafterBlockEntity::new(BlockPos::new(3, 64, -2));
        source
            .set_stack(0, ItemStack::new(2, &Item::OAK_PLANKS))
            .await;
        source
            .disabled_slots
            .store(1 << 4, std::sync::atomic::Ordering::Relaxed);
        let mut nbt = NbtCompound::new();
        source.write_nbt(&mut nbt).await;

        assert_eq!(nbt.get_int_array("disabled_slots"), Some([4].as_slice()));

        let restored = CrafterBlockEntity::from_nbt(&nbt, BlockPos::new(3, 64, -2));
        assert_eq!(restored.get_stack(0).await.item.id, Item::OAK_PLANKS.id);
        assert!(!restored.is_slot_enabled(4));
        assert!(restored.is_slot_enabled(0));
    }

    #[tokio::test]
    async fn placing_item_reenables_a_disabled_slot() {
        let crafter = CrafterBlockEntity::new(BlockPos::new(0, 64, 0));
        crafter.set_slot_disabled(4, true);
        assert!(!crafter.is_slot_enabled(4));
        crafter
            .set_stack(4, ItemStack::new(1, &Item::OAK_PLANKS))
            .await;
        assert!(crafter.is_slot_enabled(4));
    }

    #[tokio::test]
    async fn hopper_insertion_respects_disabled_and_lower_count_slots() {
        let crafter = CrafterBlockEntity::new(BlockPos::new(0, 64, 0));
        crafter
            .set_stack(0, ItemStack::new(2, &Item::OAK_PLANKS))
            .await;
        crafter
            .set_stack(1, ItemStack::new(1, &Item::OAK_PLANKS))
            .await;

        let incoming = ItemStack::new(1, &Item::OAK_PLANKS);
        assert!(!crafter.is_valid_slot_for(0, &incoming));
        assert!(!crafter.is_valid_slot_for(1, &incoming));
        assert!(crafter.is_valid_slot_for(2, &incoming));

        crafter.set_slot_disabled(2, true);
        assert!(!crafter.is_valid_slot_for(2, &incoming));
    }

    #[test]
    fn crafter_uses_vanilla_container_remainders() {
        assert_eq!(
            CrafterBlockEntity::recipe_remainder(Item::MILK_BUCKET.id).map(|item| item.id),
            Some(Item::BUCKET.id)
        );
        assert_eq!(
            CrafterBlockEntity::recipe_remainder(Item::HONEY_BOTTLE.id).map(|item| item.id),
            Some(Item::GLASS_BOTTLE.id)
        );
        assert!(CrafterBlockEntity::recipe_remainder(Item::OAK_PLANKS.id).is_none());
    }

    #[test]
    fn crafter_prefers_component_remainder_and_preserves_count() {
        use pumpkin_data::data_component::DataComponent;
        use pumpkin_data::data_component_impl::UseRemainderImpl;

        let stack = ItemStack::new_with_component(
            1,
            &Item::OAK_PLANKS,
            vec![(
                DataComponent::UseRemainder,
                Some(Box::new(UseRemainderImpl {
                    remainder: &Item::BUCKET,
                    count: 2,
                })),
            )],
        );
        let remainder = CrafterBlockEntity::recipe_remainder_for(&stack)
            .expect("component remainder should be selected");
        assert_eq!(remainder.item.id, Item::BUCKET.id);
        assert_eq!(remainder.item_count, 2);
    }
}
