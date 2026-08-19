use pumpkin_data::{
    Block, BlockDirection, BlockState, BlockStateId, entity::EntityType, tag::Taggable,
};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos};
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;

use crate::{
    block::{
        BlockBehaviour, BlockFuture, CanPlaceAtArgs, EmitsRedstonePowerArgs,
        GetComparatorOutputArgs, GetRedstonePowerArgs, OnEntityCollisionArgs, OnNeighborUpdateArgs,
        OnPlaceArgs, OnScheduledTickArgs, OnStateReplacedArgs, PlacedArgs,
    },
    entity::{EntityBase, vehicle::minecart::MinecartEntity},
    world::World,
};

use super::RailProperties;
use super::common::{
    can_place_rail_at, compute_placed_rail_shape, rail_placement_is_valid,
    update_flanking_rails_shape,
};

#[pumpkin_block("minecraft:detector_rail")]
pub struct DetectorRailBlock;

const DETECTOR_RAIL_DETECTION_BOX: BoundingBox =
    BoundingBox::new_array([0.2, 0.0, 0.2], [0.8, 0.8, 0.8]);

const fn is_minecart(entity_type: &EntityType) -> bool {
    entity_type.id == EntityType::MINECART.id
        || entity_type.id == EntityType::CHEST_MINECART.id
        || entity_type.id == EntityType::COMMAND_BLOCK_MINECART.id
        || entity_type.id == EntityType::FURNACE_MINECART.id
        || entity_type.id == EntityType::HOPPER_MINECART.id
        || entity_type.id == EntityType::SPAWNER_MINECART.id
        || entity_type.id == EntityType::TNT_MINECART.id
}

impl BlockBehaviour for DetectorRailBlock {
    fn on_place<'a>(&'a self, args: OnPlaceArgs<'a>) -> BlockFuture<'a, BlockStateId> {
        Box::pin(async move {
            let mut rail_props = RailProperties::default(args.block);
            let player_facing = args.player.get_entity().get_horizontal_facing();

            rail_props.set_waterlogged(args.replacing.water_source());
            rail_props.set_straight_shape(
                compute_placed_rail_shape(args.world, args.position, player_facing).await,
            );

            rail_props.to_state_id(args.block)
        })
    }

    fn placed<'a>(&'a self, args: PlacedArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            update_flanking_rails_shape(args.world, args.block, args.state_id, args.position).await;
            let state = args.world.get_block_state(args.position);
            self.update_pressed(args.world, args.position, state, args.block)
                .await;
        })
    }

    fn on_entity_collision<'a>(&'a self, args: OnEntityCollisionArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let props = RailProperties::new(args.state.id, args.block);
            if !props.is_powered() && is_minecart(args.entity.get_entity().entity_type) {
                self.update_pressed(args.world, args.position, args.state, args.block)
                    .await;
            }
        })
    }

    fn on_scheduled_tick<'a>(&'a self, args: OnScheduledTickArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let state = args.world.get_block_state(args.position);
            let props = RailProperties::new(state.id, args.block);
            if props.is_powered() {
                self.update_pressed(args.world, args.position, state, args.block)
                    .await;
            }
        })
    }

    fn on_neighbor_update<'a>(&'a self, args: OnNeighborUpdateArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            if !rail_placement_is_valid(args.world, args.block, args.position).await {
                args.world
                    .break_block(args.position, None, BlockFlags::NOTIFY_ALL)
                    .await;
            }
        })
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        can_place_rail_at(args.block_accessor, args.position)
    }

    fn emits_redstone_power<'a>(
        &'a self,
        _args: EmitsRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, bool> {
        Box::pin(async { true })
    }

    fn get_weak_redstone_power<'a>(
        &'a self,
        args: GetRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, u8> {
        Box::pin(async move {
            if RailProperties::new(args.state.id, args.block).is_powered() {
                15
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
            if args.direction == BlockDirection::Up
                && RailProperties::new(args.state.id, args.block).is_powered()
            {
                15
            } else {
                0
            }
        })
    }

    fn get_comparator_output<'a>(
        &'a self,
        args: GetComparatorOutputArgs<'a>,
    ) -> BlockFuture<'a, Option<u8>> {
        Box::pin(async move {
            let detection_box = DETECTOR_RAIL_DETECTION_BOX.at_pos(*args.position);
            let mut output = 0;

            for entity in args.world.get_entities_at_box(&detection_box) {
                let Some(minecart) = entity.cast_any().downcast_ref::<MinecartEntity>() else {
                    continue;
                };
                if let Some(signal) = minecart.detector_rail_comparator_output().await {
                    output = output.max(signal);
                }
            }

            Some(output)
        })
    }

    fn on_state_replaced<'a>(&'a self, args: OnStateReplacedArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            if !args.moved && RailProperties::new(args.old_state_id, args.block).is_powered() {
                args.world.update_neighbors(args.position, None).await;
                args.world
                    .update_neighbors(&args.position.down(), None)
                    .await;
            }
        })
    }
}

impl DetectorRailBlock {
    async fn update_pressed(
        &self,
        world: &std::sync::Arc<World>,
        pos: &BlockPos,
        state: &BlockState,
        block: &Block,
    ) {
        if !rail_placement_is_valid(world, block, pos).await {
            return;
        }

        let was_powered = RailProperties::new(state.id, block).is_powered();
        let detection_box = DETECTOR_RAIL_DETECTION_BOX.at_pos(*pos);
        let is_powered = world
            .get_entities_at_box(&detection_box)
            .into_iter()
            .any(|entity| is_minecart(entity.get_entity().entity_type));

        if was_powered != is_powered {
            let mut props = RailProperties::new(state.id, block);
            props.set_powered(is_powered);
            world
                .set_block_state(pos, props.to_state_id(block), BlockFlags::NOTIFY_ALL)
                .await;

            self.update_connected_rails(world, pos, &props).await;
            world.update_neighbors(pos, None).await;
            world.update_neighbors(&pos.down(), None).await;
        }

        if is_powered {
            world.schedule_block_tick(block, *pos, 20, TickPriority::Normal);
            // A cart may have changed its inventory without changing the rail's
            // powered bit.  Vanilla refreshes adjacent comparators on the
            // periodic detector-rail check as well.
            world.update_neighbors(pos, None).await;
        }
    }

    async fn update_connected_rails(
        &self,
        world: &std::sync::Arc<World>,
        pos: &BlockPos,
        props: &RailProperties,
    ) {
        for direction in props.directions() {
            let adjacent = pos.offset(direction.to_offset());
            for candidate in [adjacent, adjacent.up(), adjacent.down()] {
                let block = world.get_block(&candidate);
                if block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_RAILS) {
                    world
                        .update_neighbor(&candidate, &Block::DETECTOR_RAIL)
                        .await;
                }
            }
        }
    }
}
