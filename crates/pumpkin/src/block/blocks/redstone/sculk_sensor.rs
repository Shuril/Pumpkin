use std::sync::Arc;

use crate::block::{
    BlockBehaviour, BlockFuture, BlockMetadata, EmitsRedstonePowerArgs, GetComparatorOutputArgs,
    GetRedstonePowerArgs, GetStateForNeighborUpdateArgs, OnEntityStepArgs, OnPlaceArgs,
    OnScheduledTickArgs,
};
use crate::world::World;
use pumpkin_data::block_properties::{
    BlockProperties, CalibratedSculkSensorLikeProperties, SculkSensorLikeProperties,
    SculkSensorPhase,
};
use pumpkin_data::entity::EntityType;
use pumpkin_data::{
    Block, BlockId, BlockStateId, HorizontalFacingExt, fluid::Fluid, tag::Taggable,
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;

pub struct SculkSensorBlock;

impl BlockMetadata for SculkSensorBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::SCULK_SENSOR, BlockId::CALIBRATED_SCULK_SENSOR].into()
    }
}

impl SculkSensorBlock {
    pub async fn trigger(
        world: &Arc<World>,
        pos: &BlockPos,
        block: &Block,
        power: u8,
        vibration_frequency: i32,
        source_entity: Option<uuid::Uuid>,
    ) {
        let active_ticks = if block.id == BlockId::CALIBRATED_SCULK_SENSOR {
            10
        } else {
            30
        };
        if block.id == BlockId::SCULK_SENSOR {
            let state = world.get_block_state(pos);
            let mut props = SculkSensorLikeProperties::from_state_id(state.id, block);
            if props.sculk_sensor_phase == SculkSensorPhase::Inactive {
                props.sculk_sensor_phase = SculkSensorPhase::Active;
                props.power = power.min(15);
                world
                    .set_block_state(pos, props.to_state_id(block), BlockFlags::NOTIFY_ALL)
                    .await;
                world.update_neighbors(pos, None).await;
                if let Some(entity) = world.get_block_entity(pos)
                    && let Some(sensor) = entity
                        .as_any()
                        .downcast_ref::<crate::block::entities::sculk_sensor::SculkSensorBlockEntity>()
                {
                    *sensor.last_vibration_frequency.lock().await = vibration_frequency;
                }
                if !props.waterlogged {
                    world.play_sound(
                        pumpkin_data::sound::Sound::BlockSculkSensorClicking,
                        pumpkin_data::sound::SoundCategory::Blocks,
                        &pos.to_centered_f64(),
                    );
                }
                world.schedule_block_tick(block, *pos, active_ticks, TickPriority::Normal);
                Self::try_resonate(world, pos, vibration_frequency).await;
                Box::pin(world.emit_game_event_from(
                    *pos,
                    crate::world::game_event::GameEventKind::SculkSensorTendrilsClicking,
                    source_entity,
                ))
                .await;
            }
        } else if block.id == BlockId::CALIBRATED_SCULK_SENSOR {
            let state = world.get_block_state(pos);
            let mut props = CalibratedSculkSensorLikeProperties::from_state_id(state.id, block);
            if props.sculk_sensor_phase == SculkSensorPhase::Inactive {
                props.sculk_sensor_phase = SculkSensorPhase::Active;
                props.power = power.min(15);
                world
                    .set_block_state(pos, props.to_state_id(block), BlockFlags::NOTIFY_ALL)
                    .await;
                world.update_neighbors(pos, None).await;
                if let Some(entity) = world.get_block_entity(pos)
                    && let Some(sensor) = entity.as_any().downcast_ref::<
                        crate::block::entities::calibrated_sculk_sensor::CalibratedSculkSensorBlockEntity,
                    >()
                {
                    *sensor.last_vibration_frequency.lock().await = vibration_frequency;
                }
                if !props.waterlogged {
                    world.play_sound(
                        pumpkin_data::sound::Sound::BlockSculkSensorClicking,
                        pumpkin_data::sound::SoundCategory::Blocks,
                        &pos.to_centered_f64(),
                    );
                }
                world.schedule_block_tick(block, *pos, active_ticks, TickPriority::Normal);
                Self::try_resonate(world, pos, vibration_frequency).await;
                Box::pin(world.emit_game_event_from(
                    *pos,
                    crate::world::game_event::GameEventKind::SculkSensorTendrilsClicking,
                    source_entity,
                ))
                .await;
            }
        }
    }

    /// Amethyst blocks re-emit the matching resonance event from each face.
    /// This is deliberately performed after the sensor becomes active, so an
    /// adjacent sensor sees the same ordering as Java's `activate` method.
    async fn try_resonate(world: &Arc<World>, pos: &BlockPos, frequency: i32) {
        if !(1..=15).contains(&frequency) {
            return;
        }
        for direction in pumpkin_data::BlockDirection::all() {
            let resonator_pos = pos.offset(direction.to_offset());
            let resonator = world.get_block(&resonator_pos);
            if resonator.has_tag(&pumpkin_data::tag::Block::MINECRAFT_VIBRATION_RESONATORS) {
                // Resonance is itself a game event.  Box the recursive
                // dispatch so Rust can represent the finite event cascade;
                // sensors already active in this cascade reject duplicates,
                // matching Java's `canActivate` guard.
                Box::pin(world.emit_game_event(
                    resonator_pos,
                    crate::world::game_event::GameEventKind::Resonance(frequency as u8),
                ))
                .await;
                world.play_sound(
                    pumpkin_data::sound::Sound::BlockAmethystBlockResonate,
                    pumpkin_data::sound::SoundCategory::Blocks,
                    &resonator_pos.to_centered_f64(),
                );
            }
        }
    }
}

impl BlockBehaviour for SculkSensorBlock {
    fn on_entity_step<'a>(&'a self, args: OnEntityStepArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            // SculkSensorBlock.stepOn force-schedules a STEP vibration even
            // when the entity remains stationary on the block.  The generic
            // movement event is insufficient for that case and would also
            // incorrectly let Wardens trigger the sensor.
            if args.entity.get_entity().entity_type.id == EntityType::WARDEN.id {
                return;
            }
            let state = args.world.get_block_state(args.position);
            let can_activate = if args.block.id == BlockId::SCULK_SENSOR {
                SculkSensorLikeProperties::from_state_id(state.id, args.block).sculk_sensor_phase
                    == SculkSensorPhase::Inactive
            } else {
                CalibratedSculkSensorLikeProperties::from_state_id(state.id, args.block)
                    .sculk_sensor_phase
                    == SculkSensorPhase::Inactive
            };
            if !can_activate {
                return;
            }
            let source = args.entity.get_entity().block_pos.load();
            args.world
                .emit_game_event_from(
                    source,
                    crate::world::game_event::GameEventKind::Step,
                    Some(args.entity.get_entity().entity_uuid),
                )
                .await;
        })
    }

    fn on_place<'a>(&'a self, args: OnPlaceArgs<'a>) -> BlockFuture<'a, BlockStateId> {
        Box::pin(async move {
            if args.block.id == BlockId::CALIBRATED_SCULK_SENSOR {
                let mut props = CalibratedSculkSensorLikeProperties::default(args.block);
                props.facing = args.player.living_entity.entity.get_horizontal_facing();
                props.waterlogged = args.replacing.water_source();
                props.to_state_id(args.block)
            } else {
                let mut props = SculkSensorLikeProperties::default(args.block);
                props.waterlogged = args.replacing.water_source();
                props.to_state_id(args.block)
            }
        })
    }

    fn emits_redstone_power<'a>(
        &'a self,
        _args: EmitsRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, bool> {
        Box::pin(async move { true })
    }

    fn get_state_for_neighbor_update<'a>(
        &'a self,
        args: GetStateForNeighborUpdateArgs<'a>,
    ) -> BlockFuture<'a, BlockStateId> {
        Box::pin(async move {
            let waterlogged = if args.block.id == BlockId::SCULK_SENSOR {
                SculkSensorLikeProperties::from_state_id(args.state_id, args.block).waterlogged
            } else {
                CalibratedSculkSensorLikeProperties::from_state_id(args.state_id, args.block)
                    .waterlogged
            };
            if waterlogged {
                args.world.schedule_fluid_tick(
                    &Fluid::WATER,
                    *args.position,
                    Fluid::WATER.flow_speed as u8,
                    TickPriority::Normal,
                );
            }
            args.state_id
        })
    }

    fn get_weak_redstone_power<'a>(
        &'a self,
        args: GetRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, u8> {
        Box::pin(async move {
            if args.block.id == BlockId::SCULK_SENSOR {
                let props = SculkSensorLikeProperties::from_state_id(args.state.id, args.block);
                if props.sculk_sensor_phase == SculkSensorPhase::Active {
                    props.power
                } else {
                    0
                }
            } else if args.block.id == BlockId::CALIBRATED_SCULK_SENSOR {
                let props =
                    CalibratedSculkSensorLikeProperties::from_state_id(args.state.id, args.block);
                if props.sculk_sensor_phase == SculkSensorPhase::Active
                    && args.direction != props.facing.to_block_direction()
                {
                    props.power
                } else {
                    0
                }
            } else {
                0
            }
        })
    }

    fn get_strong_redstone_power<'a>(
        &'a self,
        args: GetRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, u8> {
        Box::pin(async move {
            if args.block.id == BlockId::SCULK_SENSOR {
                if args.direction != pumpkin_data::BlockDirection::Up {
                    return 0;
                }
                let props = SculkSensorLikeProperties::from_state_id(args.state.id, args.block);
                if props.sculk_sensor_phase == SculkSensorPhase::Active {
                    props.power
                } else {
                    0
                }
            } else {
                if args.direction != pumpkin_data::BlockDirection::Up {
                    0
                } else {
                    let props = CalibratedSculkSensorLikeProperties::from_state_id(
                        args.state.id,
                        args.block,
                    );
                    if props.sculk_sensor_phase == SculkSensorPhase::Active {
                        props.power
                    } else {
                        0
                    }
                }
            }
        })
    }

    fn get_comparator_output<'a>(
        &'a self,
        args: GetComparatorOutputArgs<'a>,
    ) -> BlockFuture<'a, Option<u8>> {
        Box::pin(async move {
            let active = if args.block.id == BlockId::SCULK_SENSOR {
                SculkSensorLikeProperties::from_state_id(args.state.id, args.block)
                    .sculk_sensor_phase
                    == SculkSensorPhase::Active
            } else {
                CalibratedSculkSensorLikeProperties::from_state_id(args.state.id, args.block)
                    .sculk_sensor_phase
                    == SculkSensorPhase::Active
            };
            if !active {
                return Some(0);
            }
            let frequency = args.world.get_block_entity(args.position).and_then(|entity| {
                if args.block.id == BlockId::SCULK_SENSOR {
                    entity
                        .as_any()
                        .downcast_ref::<crate::block::entities::sculk_sensor::SculkSensorBlockEntity>()
                        .map(|sensor| sensor.last_vibration_frequency.try_lock().ok().map(|v| *v))
                        .flatten()
                } else {
                    entity
                        .as_any()
                        .downcast_ref::<crate::block::entities::calibrated_sculk_sensor::CalibratedSculkSensorBlockEntity>()
                        .map(|sensor| sensor.last_vibration_frequency.try_lock().ok().map(|v| *v))
                        .flatten()
                }
            });
            Some(frequency.unwrap_or(0).clamp(0, 15) as u8)
        })
    }

    fn on_scheduled_tick<'a>(&'a self, args: OnScheduledTickArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let state = args.world.get_block_state(args.position);
            if args.block.id == BlockId::SCULK_SENSOR {
                let mut props = SculkSensorLikeProperties::from_state_id(state.id, args.block);
                match props.sculk_sensor_phase {
                    SculkSensorPhase::Active => {
                        props.sculk_sensor_phase = SculkSensorPhase::Cooldown;
                        props.power = 0;
                        args.world
                            .set_block_state(
                                args.position,
                                props.to_state_id(args.block),
                                BlockFlags::NOTIFY_ALL,
                            )
                            .await;
                        args.world.schedule_block_tick(
                            args.block,
                            *args.position,
                            10,
                            TickPriority::Normal,
                        );
                        args.world.update_neighbors(args.position, None).await;
                    }
                    SculkSensorPhase::Cooldown => {
                        props.sculk_sensor_phase = SculkSensorPhase::Inactive;
                        props.power = 0;
                        args.world
                            .set_block_state(
                                args.position,
                                props.to_state_id(args.block),
                                BlockFlags::NOTIFY_ALL,
                            )
                            .await;
                        if !props.waterlogged {
                            args.world.play_sound(
                                pumpkin_data::sound::Sound::BlockSculkSensorClickingStop,
                                pumpkin_data::sound::SoundCategory::Blocks,
                                &args.position.to_centered_f64(),
                            );
                        }
                        args.world.update_neighbors(args.position, None).await;
                    }
                    SculkSensorPhase::Inactive => {}
                }
            } else if args.block.id == BlockId::CALIBRATED_SCULK_SENSOR {
                let mut props =
                    CalibratedSculkSensorLikeProperties::from_state_id(state.id, args.block);
                match props.sculk_sensor_phase {
                    SculkSensorPhase::Active => {
                        props.sculk_sensor_phase = SculkSensorPhase::Cooldown;
                        props.power = 0;
                        args.world
                            .set_block_state(
                                args.position,
                                props.to_state_id(args.block),
                                BlockFlags::NOTIFY_ALL,
                            )
                            .await;
                        args.world.schedule_block_tick(
                            args.block,
                            *args.position,
                            10,
                            TickPriority::Normal,
                        );
                        args.world.update_neighbors(args.position, None).await;
                    }
                    SculkSensorPhase::Cooldown => {
                        props.sculk_sensor_phase = SculkSensorPhase::Inactive;
                        props.power = 0;
                        args.world
                            .set_block_state(
                                args.position,
                                props.to_state_id(args.block),
                                BlockFlags::NOTIFY_ALL,
                            )
                            .await;
                        if !props.waterlogged {
                            args.world.play_sound(
                                pumpkin_data::sound::Sound::BlockSculkSensorClickingStop,
                                pumpkin_data::sound::SoundCategory::Blocks,
                                &args.position.to_centered_f64(),
                            );
                        }
                        args.world.update_neighbors(args.position, None).await;
                    }
                    SculkSensorPhase::Inactive => {}
                }
            }
        })
    }
}
