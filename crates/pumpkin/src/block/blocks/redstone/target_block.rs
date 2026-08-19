use pumpkin_data::block_properties::{BlockProperties, LightWeightedPressurePlateLikeProperties};
use pumpkin_data::entity::EntityType;
use pumpkin_macros::pumpkin_block;
use pumpkin_world::{tick::TickPriority, world::BlockFlags};

use crate::block::{
    BlockBehaviour, BlockFuture, EmitsRedstonePowerArgs, GetRedstonePowerArgs, OnProjectileHitArgs,
    OnScheduledTickArgs, PlacedArgs,
};

#[pumpkin_block("minecraft:target")]
pub struct TargetBlock;

impl BlockBehaviour for TargetBlock {
    fn placed<'a>(&'a self, args: PlacedArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let props =
                LightWeightedPressurePlateLikeProperties::from_state_id(args.state_id, args.block);
            if props.power != 0
                && !args
                    .world
                    .is_block_tick_scheduled(args.position, args.block)
            {
                args.world.schedule_block_tick(
                    args.block,
                    *args.position,
                    18u32,
                    TickPriority::Normal,
                );
            }
        })
    }

    fn emits_redstone_power<'a>(
        &'a self,
        _args: EmitsRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, bool> {
        Box::pin(async move { true })
    }

    fn get_weak_redstone_power<'a>(
        &'a self,
        args: GetRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, u8> {
        Box::pin(async move {
            LightWeightedPressurePlateLikeProperties::from_state_id(args.state.id, args.block).power
        })
    }

    fn get_strong_redstone_power<'a>(
        &'a self,
        args: GetRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, u8> {
        self.get_weak_redstone_power(args)
    }

    fn on_scheduled_tick<'a>(&'a self, args: OnScheduledTickArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let state = args.world.get_block_state(args.position);
            if args.world.get_block(args.position) != args.block {
                return;
            }
            let props =
                LightWeightedPressurePlateLikeProperties::from_state_id(state.id, args.block);
            if props.power != 0 {
                let mut reset = props;
                reset.power = 0;
                args.world
                    .set_block_state(
                        args.position,
                        reset.to_state_id(args.block),
                        BlockFlags::NOTIFY_LISTENERS,
                    )
                    .await;
            }
        })
    }

    fn on_projectile_hit<'a>(&'a self, args: OnProjectileHitArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let crate::entity::projectile::ProjectileHit::Block { face, hit_pos, .. } = args.hit
            else {
                return;
            };
            let strength =
                Self::redstone_strength(args.block, args.state, *face, *hit_pos, args.position);
            if !args
                .world
                .is_block_tick_scheduled(args.position, args.block)
            {
                let mut props = LightWeightedPressurePlateLikeProperties::from_state_id(
                    args.state.id,
                    args.block,
                );
                props.power = strength;
                args.world
                    .set_block_state(
                        args.position,
                        props.to_state_id(args.block),
                        BlockFlags::NOTIFY_LISTENERS,
                    )
                    .await;

                let delay = Self::activation_ticks(args.projectile.get_entity().entity_type);
                args.world.schedule_block_tick(
                    args.block,
                    *args.position,
                    delay,
                    TickPriority::Normal,
                );
            }

            if let Some(player) = args
                .projectile
                .projectile_owner_id()
                .and_then(|id| args.world.get_player_by_id(id))
            {
                player
                    .increment_stat(
                        crate::entity::player::statistics::StatisticCategory::Custom,
                        crate::entity::player::statistics::CustomStatistic::TargetHit as i32,
                        1,
                    )
                    .await;
                if strength >= 15
                    && player
                        .position()
                        .squared_distance_to_xz(hit_pos.x, hit_pos.z)
                        >= 30.0 * 30.0
                {
                    player
                        .trigger_advancement(
                            crate::entity::player::advancement::trigger::AdvancementTrigger::Bullseye,
                        )
                        .await;
                }
            }
        })
    }
}

impl TargetBlock {
    /// TargetBlock uses the long activation window for every
    /// `AbstractArrow` projectile.  Mojang's `ThrownTrident` extends
    /// `AbstractArrow`, so tridents share the 20-tick window with arrows and
    /// spectral arrows; snowballs/eggs and other thrown projectiles use eight.
    #[must_use]
    pub const fn activation_ticks(entity_type: &EntityType) -> u32 {
        if entity_type.id == EntityType::ARROW.id
            || entity_type.id == EntityType::SPECTRAL_ARROW.id
            || entity_type.id == EntityType::TRIDENT.id
        {
            20
        } else {
            8
        }
    }

    /// Vanilla's distance-to-center mapping, kept pure for differential tests.
    #[must_use]
    pub fn redstone_strength(
        _block: &pumpkin_data::Block,
        _state: &pumpkin_data::BlockState,
        face: pumpkin_data::BlockDirection,
        hit_pos: pumpkin_util::math::vector3::Vector3<f64>,
        block_pos: &pumpkin_util::math::position::BlockPos,
    ) -> u8 {
        let local = hit_pos.sub(&block_pos.0.to_f64());
        let distance = match face {
            pumpkin_data::BlockDirection::Up | pumpkin_data::BlockDirection::Down => {
                (local.x - 0.5).abs().max((local.z - 0.5).abs())
            }
            pumpkin_data::BlockDirection::North | pumpkin_data::BlockDirection::South => {
                (local.x - 0.5).abs().max((local.y - 0.5).abs())
            }
            pumpkin_data::BlockDirection::West | pumpkin_data::BlockDirection::East => {
                (local.y - 0.5).abs().max((local.z - 0.5).abs())
            }
        };
        ((15.0 * (1.0 - (distance / 0.5))).clamp(0.0, 15.0).ceil() as u8).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::TargetBlock;
    use pumpkin_data::BlockDirection;
    use pumpkin_data::entity::EntityType;
    use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

    #[test]
    fn target_power_uses_the_hit_face_plane() {
        let pos = BlockPos::new(0, 0, 0);
        let center = Vector3::new(0.5, 0.5, 0.5);
        assert_eq!(
            TargetBlock::redstone_strength(
                &pumpkin_data::Block::TARGET,
                &pumpkin_data::Block::TARGET.default_state,
                BlockDirection::North,
                center,
                &pos
            ),
            15
        );
        assert_eq!(
            TargetBlock::redstone_strength(
                &pumpkin_data::Block::TARGET,
                &pumpkin_data::Block::TARGET.default_state,
                BlockDirection::North,
                Vector3::new(0.0, 0.5, 0.5),
                &pos
            ),
            1
        );
        // The coordinate along the face normal does not affect the signal.
        assert_eq!(
            TargetBlock::redstone_strength(
                &pumpkin_data::Block::TARGET,
                &pumpkin_data::Block::TARGET.default_state,
                BlockDirection::North,
                Vector3::new(0.5, 0.5, 0.0),
                &pos
            ),
            15
        );
    }

    #[test]
    fn target_activation_window_matches_vanilla_projectile_classes() {
        assert_eq!(TargetBlock::activation_ticks(&EntityType::ARROW), 20);
        assert_eq!(
            TargetBlock::activation_ticks(&EntityType::SPECTRAL_ARROW),
            20
        );
        assert_eq!(TargetBlock::activation_ticks(&EntityType::TRIDENT), 20);
        assert_eq!(TargetBlock::activation_ticks(&EntityType::SNOWBALL), 8);
    }
}
