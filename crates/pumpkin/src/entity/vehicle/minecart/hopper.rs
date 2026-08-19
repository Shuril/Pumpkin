use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use crate::block::entities::hopper::HopperBlockEntity;
use crate::entity::{Entity, player::Player};
use pumpkin_data::translation;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::text::TextComponent;

use super::container::{self, MinecartInventory};

pub(super) struct HopperMinecart {
    enabled: AtomicBool,
    transfer_cooldown: AtomicI32,
    inventory: Arc<MinecartInventory>,
}

impl HopperMinecart {
    pub(super) fn new() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            transfer_cooldown: AtomicI32::new(-1),
            inventory: Arc::new(MinecartInventory::new(5)),
        }
    }

    pub(super) const fn inventory(&self) -> &Arc<MinecartInventory> {
        &self.inventory
    }

    pub(super) fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub(super) async fn tick(&self, entity: &Entity) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }

        // Hopper minecarts use the same eight-tick transfer cadence as a
        // stationary hopper.  The decrement and gate are one atomic update so
        // an entity tick cannot accidentally run twice after a reload.
        if self.transfer_cooldown.fetch_sub(1, Ordering::Relaxed) > 0 {
            return;
        }
        self.transfer_cooldown.store(0, Ordering::Relaxed);

        let world = entity.world.load();
        let pos = entity.pos.load();
        let source_pos = BlockPos::floored(pos.x, pos.y + 1.5, pos.z);
        if let Some(block_entity) = world.get_block_entity(&source_pos)
            && let Some(source) = block_entity.get_inventory()
        {
            for slot in 0..source.size() {
                let stack = source.get_stack(slot).await;
                if stack.is_empty()
                    || !source.can_extract_to_hopper(
                        self.inventory.as_ref(),
                        slot,
                        &stack,
                        pumpkin_data::BlockDirection::Down,
                    )
                {
                    continue;
                }
                let one = stack.copy_with_count(1);
                if HopperBlockEntity::add_one_item(
                    source.as_ref(),
                    self.inventory.as_ref(),
                    one,
                    pumpkin_data::BlockDirection::Down,
                )
                .await
                {
                    source.remove_stack_specific(slot, 1).await;
                    self.transfer_cooldown.store(8, Ordering::Relaxed);
                    return;
                }
            }
            return;
        }

        let suction_box = BoundingBox::new(
            Vector3::new(pos.x - 0.5, pos.y + 0.6875, pos.z - 0.5),
            Vector3::new(pos.x + 0.5, pos.y + 2.0, pos.z + 0.5),
        );
        if self.pick_up_item(entity, &suction_box).await {
            self.transfer_cooldown.store(8, Ordering::Relaxed);
            return;
        }
        let cart_box = entity.bounding_box.load().expand(0.25, 0.0, 0.25);
        if self.pick_up_item(entity, &cart_box).await {
            self.transfer_cooldown.store(8, Ordering::Relaxed);
        }
    }

    async fn pick_up_item(&self, entity: &Entity, search_box: &BoundingBox) -> bool {
        let world = entity.world.load();
        for entity in world.get_entities_at_box(search_box) {
            let Some(item) = entity.get_item_entity() else {
                continue;
            };
            let mut stack = item.get_item_stack().lock().await;
            if stack.is_empty() {
                continue;
            }
            let backup = stack.clone();
            let one = stack.split(1);
            if HopperBlockEntity::add_one_item(
                self.inventory.as_ref(),
                self.inventory.as_ref(),
                one,
                pumpkin_data::BlockDirection::Down,
            )
            .await
            {
                if stack.is_empty() {
                    item.get_entity().remove().await;
                }
                return true;
            }
            *stack = backup;
        }
        false
    }

    pub(super) async fn interact(&self, entity: &Entity, player: &Arc<Player>) -> bool {
        container::open(
            entity,
            player,
            &self.inventory,
            TextComponent::translate_cross(
                translation::java::ENTITY_MINECRAFT_HOPPER_MINECART,
                translation::bedrock::ENTITY_HOPPER_MINECART_NAME,
                [],
            ),
            true,
        )
        .await
    }

    pub(super) async fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.inventory.write_nbt(nbt).await;
        nbt.put_bool("Enabled", self.enabled.load(Ordering::Relaxed));
        nbt.put_int(
            "TransferCooldown",
            self.transfer_cooldown.load(Ordering::Relaxed),
        );
    }

    pub(super) async fn read_nbt(&self, nbt: &NbtCompound) {
        self.inventory.read_nbt(nbt).await;
        self.enabled
            .store(nbt.get_bool("Enabled").unwrap_or(true), Ordering::Relaxed);
        self.transfer_cooldown.store(
            nbt.get_int("TransferCooldown").unwrap_or(-1),
            Ordering::Relaxed,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::HopperMinecart;
    use pumpkin_world::inventory::Inventory;

    #[test]
    fn hopper_minecart_inventory_has_five_slots() {
        assert_eq!(HopperMinecart::new().inventory.size(), 5);
    }
}
