use pumpkin_data::block_properties::{BlockProperties, PistonType};
use pumpkin_data::{Block, FacingExt};
use pumpkin_macros::pumpkin_block;
use pumpkin_world::world::{BlockAccessor, BlockFlags};

use crate::block::{
    BlockBehaviour, BlockFuture, BrokenArgs, CanPlaceAtArgs, GetStateForNeighborUpdateArgs,
    OnNeighborUpdateArgs,
};

use super::piston::PistonProps;

pub(crate) type PistonHeadProperties = pumpkin_data::block_properties::PistonHeadLikeProperties;

#[pumpkin_block("minecraft:piston_head")]
pub struct PistonHeadBlock;

impl BlockBehaviour for PistonHeadBlock {
    fn broken<'a>(&'a self, args: BrokenArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let props = PistonHeadProperties::from_state_id(args.state.id, &Block::PISTON_HEAD);
            let pos = args
                .position
                .offset(props.facing.opposite().to_block_direction().to_offset());
            let (new_block, new_state) = args.world.get_block_and_state_id(&pos);
            let expected_base = match props.r#type {
                PistonType::Normal => &Block::PISTON,
                PistonType::Sticky => &Block::STICKY_PISTON,
            };
            if expected_base == new_block {
                let props = PistonProps::from_state_id(new_state, new_block);
                if props.extended
                    && props.facing.to_block_direction()
                        == PistonHeadProperties::from_state_id(args.state.id, &Block::PISTON_HEAD)
                            .facing
                            .to_block_direction()
                {
                    args.world
                        .break_block(&pos, Some(args.player.clone()), BlockFlags::SKIP_DROPS)
                        .await;
                }
            }
        })
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        Self::can_survive(args.block_accessor, args.position, args.state.id)
    }

    fn get_state_for_neighbor_update<'a>(
        &'a self,
        args: GetStateForNeighborUpdateArgs<'a>,
    ) -> BlockFuture<'a, pumpkin_data::BlockStateId> {
        Box::pin(async move {
            let props = PistonHeadProperties::from_state_id(args.state_id, args.block);
            if args.direction.opposite() == props.facing.to_block_direction()
                && !Self::can_survive(args.world, args.position, args.state_id)
            {
                return Block::AIR.default_state.id;
            }
            args.state_id
        })
    }

    fn on_neighbor_update<'a>(&'a self, args: OnNeighborUpdateArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let head_state_id = args.world.get_block_state_id(args.position);
            let head_props =
                PistonHeadProperties::from_state_id(head_state_id, &Block::PISTON_HEAD);
            let piston_pos = args.position.offset(
                head_props
                    .facing
                    .opposite()
                    .to_block_direction()
                    .to_offset(),
            );
            if Self::can_survive(args.world.as_ref(), args.position, head_state_id) {
                args.world
                    .update_neighbor(&piston_pos, args.source_block)
                    .await;
            }
        })
    }
}

impl PistonHeadBlock {
    fn can_survive(
        world: &dyn BlockAccessor,
        position: &pumpkin_util::math::position::BlockPos,
        state_id: pumpkin_data::BlockStateId,
    ) -> bool {
        let props = PistonHeadProperties::from_state_id(state_id, &Block::PISTON_HEAD);
        let facing = props.facing.to_block_direction();
        let base_pos = position.offset(facing.opposite().to_offset());
        let (base, base_state) = world.get_block_and_state(&base_pos);

        if base == &Block::MOVING_PISTON {
            let moving =
                super::piston_extension::MovingPistonProps::from_state_id(base_state.id, base);
            return moving.facing == props.facing;
        }

        let expected_base = match props.r#type {
            PistonType::Normal => &Block::PISTON,
            PistonType::Sticky => &Block::STICKY_PISTON,
        };
        if base != expected_base {
            return false;
        }

        let base_props = PistonProps::from_state_id(base_state.id, base);
        base_props.extended && base_props.facing == props.facing
    }
}
