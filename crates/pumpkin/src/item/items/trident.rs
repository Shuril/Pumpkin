use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::entity::player::Player;
use crate::entity::projectile::arrow::ArrowPickup;
use crate::entity::projectile::trident::TridentEntity;
use crate::entity::{Entity, EntityBase};
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_util::GameMode;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::inventory::Inventory;

pub struct TridentItem;

#[must_use]
const fn riptide_strength(level: u32) -> f64 {
    // Vanilla LevelBasedValue.perLevel(1.5f, 0.75f): base value applies to
    // level I and the increment applies only above the first level.
    1.5 + ((level as f64) - 1.0) * 0.75
}

impl ItemMetadata for TridentItem {
    fn ids() -> Box<[u16]> {
        [Item::TRIDENT.id].into()
    }
}

impl ItemBehaviour for TridentItem {
    fn normal_use<'a>(
        &'a self,
        _item: &'a Item,
        player: &'a Player,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let inventory = player.inventory();
            let stack = inventory.held_item().await;

            // `TridentItem.use` rejects an already-last-durability trident and
            // rejects Riptide while the player is neither in water/rain nor
            // eligible to launch from a vehicle.  Doing this at start-use is
            // important: vanilla never enters the use animation in these
            // cases, so release cannot accidentally throw or spin.
            if stack.next_damage_will_break() {
                return;
            }
            if stack
                .get_data_component::<pumpkin_data::data_component_impl::EnchantmentsImpl>()
                .is_some_and(|enchantments| {
                    enchantments.enchantment.iter().any(|(enchantment, level)| {
                        **enchantment == pumpkin_data::Enchantment::RIPTIDE && *level > 0
                    })
                })
            {
                let in_water = player
                    .get_entity()
                    .touching_water
                    .load(std::sync::atomic::Ordering::Relaxed)
                    || player
                        .world()
                        .is_raining_at(&player.position().to_block_pos())
                        .await;
                if !in_water || player.get_entity().has_vehicle().await {
                    return;
                }
            }

            player
                .living_entity
                .set_active_hand(pumpkin_util::Hand::Right, stack, 72000)
                .await;
        })
    }

    fn on_stopped_using<'a>(
        &'a self,
        _stack: &'a ItemStack,
        player: &'a Player,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let use_ticks = player
                .living_entity
                .item_use_time
                .load(std::sync::atomic::Ordering::Relaxed);
            let use_ticks = 72000 - use_ticks;

            if use_ticks < 10 {
                return;
            }

            let world = player.world();
            let stack_guard = player.inventory().held_item().await;

            if stack_guard.next_damage_will_break() {
                player.living_entity.clear_active_hand().await;
                return;
            }

            // Check Riptide level
            let mut riptide_level = 0u32;
            if let Some(enchantments) = stack_guard
                .get_data_component::<pumpkin_data::data_component_impl::EnchantmentsImpl>(
            ) {
                for (enchantment, level) in enchantments.enchantment.iter() {
                    if **enchantment == pumpkin_data::Enchantment::RIPTIDE {
                        riptide_level = *level as u32;
                    }
                }
            }

            if riptide_level > 0 {
                // Vanilla's `isInWaterOrRain` accepts flowing water and rain
                // at the player's position; checking only the still-water
                // block state incorrectly rejects Riptide launches in rivers
                // and during storms.
                let player_pos = player.position().to_block_pos();
                let in_water = player
                    .get_entity()
                    .touching_water
                    .load(std::sync::atomic::Ordering::Relaxed)
                    || world.is_raining_at(&player_pos).await;
                if !in_water || player.get_entity().has_vehicle().await {
                    player.living_entity.clear_active_hand().await;
                    return;
                }

                // Vanilla's `LevelBasedValue.perLevel(1.5, 0.75)` yields
                // 1.5/2.25/3.0 for Riptide I/II/III.  The previous
                // `1.0 + level * 0.75` was one quarter-block too strong at
                // every level.
                let f = riptide_strength(riptide_level);
                let (yaw, pitch) = player.rotation();
                let f_yaw = f32::to_radians(yaw);
                let f_pitch = f32::to_radians(pitch);

                let vx = f64::from(-f32::sin(f_yaw) * f32::cos(f_pitch));
                let vy = f64::from(-f32::sin(f_pitch));
                let vz = f64::from(f32::cos(f_yaw) * f32::cos(f_pitch));

                let sq = (vx * vx + vy * vy + vz * vz).sqrt();
                if sq > 0.0 {
                    let mult = f / sq;
                    player.living_entity.entity.velocity.store(Vector3::new(
                        vx * mult,
                        vy * mult,
                        vz * mult,
                    ));
                }

                // `TridentItem.releaseUsing` gives a grounded player the
                // vanilla 1.2-block upward SELF movement impulse.  Pumpkin's
                // player movement loop consumes velocity directly, so adding
                // the impulse here is the equivalent collision-safe input.
                if player
                    .get_entity()
                    .on_ground
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    player
                        .get_entity()
                        .add_velocity(Vector3::new(0.0, 1.2, 0.0));
                }

                // Player#autoSpinAttackTicks drives the SpinAttack pose and
                // collision shape for the duration of a riptide launch.
                player
                    .auto_spin_attack_ticks
                    .store(20, std::sync::atomic::Ordering::Relaxed);

                let riptide_sound = match riptide_level {
                    1 => Sound::ItemTridentRiptide1,
                    2 => Sound::ItemTridentRiptide2,
                    _ => Sound::ItemTridentRiptide3,
                };
                world.play_sound(
                    riptide_sound,
                    pumpkin_data::sound::SoundCategory::Players,
                    &player.position(),
                );

                player.damage_held_item(1).await;
                player.living_entity.clear_active_hand().await;
                return;
            }

            // Normal throw - spawn thrown trident
            let (yaw, pitch) = player.rotation();
            let entity = Entity::new(world.clone(), player.position(), &EntityType::TRIDENT);
            let trident = TridentEntity::new_shot(
                entity,
                player.get_entity(),
                stack_guard.clone(),
                ArrowPickup::Allowed,
            );
            trident.set_velocity_from_rotation(pitch, yaw, 0.0, 2.5, 1.0);
            world.spawn_entity(Arc::new(trident)).await;

            world.play_sound(
                Sound::ItemTridentThrow,
                pumpkin_data::sound::SoundCategory::Players,
                &player.position(),
            );

            if player.gamemode.load() != GameMode::Creative {
                let inventory = player.inventory();
                let selected_slot = inventory.get_selected_slot() as usize;

                let main_hand_item = inventory.get_stack(selected_slot).await;
                if main_hand_item.item.id == Item::TRIDENT.id {
                    inventory
                        .set_stack(selected_slot, ItemStack::EMPTY.clone())
                        .await;
                    player
                        .sync_hand_slot(selected_slot, ItemStack::EMPTY.clone())
                        .await;
                } else {
                    let off_hand_slot =
                        pumpkin_inventory::player::player_inventory::PlayerInventory::OFF_HAND_SLOT;
                    let off_hand_item = inventory.get_stack(off_hand_slot).await;
                    if off_hand_item.item.id == Item::TRIDENT.id {
                        inventory
                            .set_stack(off_hand_slot, ItemStack::EMPTY.clone())
                            .await;
                        player
                            .sync_hand_slot(off_hand_slot, ItemStack::EMPTY.clone())
                            .await;
                    }
                }
            }

            player.living_entity.clear_active_hand().await;
        })
    }

    fn can_mine(&self, player: &Player) -> bool {
        player.gamemode.load() != GameMode::Creative
    }

    fn get_use_duration(&self) -> i32 {
        72000
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::riptide_strength;

    #[test]
    fn riptide_strength_matches_vanilla_level_based_value() {
        assert_eq!(riptide_strength(0), 0.75);
        assert_eq!(riptide_strength(1), 1.5);
        assert_eq!(riptide_strength(2), 2.25);
        assert_eq!(riptide_strength(3), 3.0);
        assert_eq!(riptide_strength(255), 192.0);
    }
}
