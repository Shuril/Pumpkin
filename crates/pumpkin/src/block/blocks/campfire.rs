use pumpkin_data::{
    Block, BlockDirection, BlockStateId, Enchantment,
    block_properties::{BlockProperties, CampfireLikeProperties},
    damage::DamageType,
    data_component_impl::EquipmentSlot,
    effect::StatusEffect,
    fluid::Fluid,
};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;

use crate::block::entities::campfire::CampfireBlockEntity;
use crate::{
    block::{
        BlockBehaviour, BlockFuture, BlockIsReplacing, GetStateForNeighborUpdateArgs,
        OnEntityCollisionArgs, OnPlaceArgs, OnProjectileHitArgs, PlacedArgs, UseWithItemArgs,
        registry::BlockActionResult,
    },
    entity::EntityBase,
};
use std::sync::Arc;

#[pumpkin_block_from_tag("minecraft:campfires")]
pub struct CampfireBlock;

impl BlockBehaviour for CampfireBlock {
    fn placed<'a>(&'a self, args: PlacedArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let entity = CampfireBlockEntity::new(*args.position);
            args.world.add_block_entity(Arc::new(entity));
        })
    }

    fn use_with_item<'a>(
        &'a self,
        args: UseWithItemArgs<'a>,
    ) -> BlockFuture<'a, BlockActionResult> {
        Box::pin(async move {
            if !CampfireLikeProperties::from_state_id(
                args.world.get_block_state(args.position).id,
                args.block,
            )
            .lit
            {
                return BlockActionResult::PassToDefaultBlockAction;
            }
            let Some(recipe) = pumpkin_data::recipes::get_cooking_recipe_with_ingredient(
                args.item_stack.get_item(),
                pumpkin_data::recipes::CookingRecipeKind::CampfireCooking,
            ) else {
                return BlockActionResult::PassToDefaultBlockAction;
            };
            let Some(entity) = args.world.get_block_entity(args.position) else {
                return BlockActionResult::Fail;
            };
            let Some(campfire) = entity.as_any().downcast_ref::<CampfireBlockEntity>() else {
                return BlockActionResult::Fail;
            };
            let mut slot = None;
            for (index, item) in campfire.items.iter().enumerate() {
                if item.lock().await.is_empty() {
                    slot = Some(index);
                    break;
                }
            }
            let Some(slot) = slot else {
                return BlockActionResult::Fail;
            };
            let input = args
                .item_stack
                .split_unless_creative(args.player.gamemode.load(), 1);
            *campfire.items[slot].lock().await = input;
            *campfire.cooking_times[slot].lock().await = 0;
            *campfire.cooking_total_times[slot].lock().await = recipe.cookingtime.max(1);
            campfire
                .dirty
                .store(true, std::sync::atomic::Ordering::Relaxed);
            args.player
                .increment_stat(
                    pumpkin_data::statistic::StatisticCategory::Custom,
                    pumpkin_data::statistic::CustomStatistic::InteractWithCampfire as i32,
                    1,
                )
                .await;
            BlockActionResult::Success
        })
    }

    fn on_entity_collision<'a>(&'a self, args: OnEntityCollisionArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            if CampfireLikeProperties::from_state_id(args.state.id, args.block).lit
                && let Some(living_entity) = args.entity.get_living_entity()
            {
                let has_frost_walker_enchantment = {
                    let equipment = living_entity.entity_equipment.lock().await;
                    equipment
                        .equipment
                        .get(&EquipmentSlot::FEET)
                        .is_some_and(|boots| {
                            boots.get_enchantment_level(&Enchantment::FROST_WALKER) != 0
                        })
                };
                let has_fire_res = living_entity
                    .get_effect(&StatusEffect::FIRE_RESISTANCE)
                    .await
                    .is_some();
                if has_frost_walker_enchantment || has_fire_res {
                    //campfire burning doesn't work if entity's boots has frost walker enchantment or entity has fire resistance. source: https://minecraft.wiki/w/Campfire#Damage
                    return;
                }
                let damage_amount = if args.block == &Block::SOUL_CAMPFIRE {
                    2.0
                } else {
                    1.0
                };
                args.entity
                    .damage(args.entity, damage_amount, DamageType::CAMPFIRE)
                    .await;
            }
        })
    }

    fn on_projectile_hit<'a>(&'a self, args: OnProjectileHitArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let properties = CampfireLikeProperties::from_state_id(args.state.id, args.block);
            let on_fire = args
                .projectile
                .get_entity()
                .fire_ticks
                .load(std::sync::atomic::Ordering::Relaxed)
                > 0;
            let may_interact = args
                .projectile
                .projectile_owner_id()
                .and_then(|owner_id| args.world.get_player_by_id(owner_id))
                .is_none_or(|player| player.can_interact_with_block_at(args.position, 0.0));
            if !can_ignite_from_projectile(
                properties.lit,
                properties.waterlogged,
                on_fire,
                may_interact,
            ) {
                return;
            }

            let mut lit = properties;
            lit.lit = true;
            args.world
                .set_block_state(
                    args.position,
                    lit.to_state_id(args.block),
                    BlockFlags::NOTIFY_ALL,
                )
                .await;
        })
    }

    fn on_place<'a>(&'a self, args: OnPlaceArgs<'a>) -> BlockFuture<'a, BlockStateId> {
        Box::pin(async move {
            let is_replacing_water = matches!(args.replacing, BlockIsReplacing::Water(_));
            let mut props =
                CampfireLikeProperties::from_state_id(args.block.default_state.id, args.block);
            props.waterlogged = is_replacing_water;
            props.signal_fire =
                is_signal_fire_base_block(args.world.get_block(&args.position.down()));
            props.lit = !is_replacing_water;
            props.facing = args.player.get_entity().get_horizontal_facing();
            props.to_state_id(args.block)
        })
    }

    fn get_state_for_neighbor_update<'a>(
        &'a self,
        args: GetStateForNeighborUpdateArgs<'a>,
    ) -> BlockFuture<'a, BlockStateId> {
        Box::pin(async move {
            let mut props = CampfireLikeProperties::from_state_id(args.state_id, args.block);
            if props.waterlogged {
                props.lit = false;
                args.world.schedule_fluid_tick(
                    &Fluid::WATER,
                    *args.position,
                    Fluid::WATER.flow_speed as u8,
                    TickPriority::Normal,
                );
            }

            if args.direction == BlockDirection::Down {
                props.signal_fire =
                    is_signal_fire_base_block(args.world.get_block(args.neighbor_position));
            }

            props.to_state_id(args.block)
        })
    }
}

fn is_signal_fire_base_block(block: &Block) -> bool {
    block == &Block::HAY_BLOCK
}

#[inline]
fn can_ignite_from_projectile(
    lit: bool,
    waterlogged: bool,
    projectile_on_fire: bool,
    may_interact: bool,
) -> bool {
    projectile_on_fire && may_interact && !lit && !waterlogged
}

#[cfg(test)]
mod tests {
    use super::can_ignite_from_projectile;

    #[test]
    fn only_unlit_dry_campfires_are_ignited_by_fire_projectiles() {
        assert!(can_ignite_from_projectile(false, false, true, true));
        assert!(!can_ignite_from_projectile(true, false, true, true));
        assert!(!can_ignite_from_projectile(false, true, true, true));
        assert!(!can_ignite_from_projectile(false, false, false, true));
        assert!(!can_ignite_from_projectile(false, false, true, false));
    }
}
