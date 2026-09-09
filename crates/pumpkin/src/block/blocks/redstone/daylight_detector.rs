use std::sync::Arc;

use crate::block::entities::daylight_detector::DaylightDetectorBlockEntity;
use pumpkin_data::{Block, block_properties::BlockProperties};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use crate::block::{
    BlockActionResult, BlockBehaviour, BlockFuture, BrokenArgs, EmitsRedstonePowerArgs,
    GetRedstonePowerArgs, NormalUseArgs, OnScheduledTickArgs, PlacedArgs, RandomTickArgs,
};
use crate::world::World;
use crate::world::game_event::GameEventKind;

type DaylightDetectorProperties = pumpkin_data::block_properties::DaylightDetectorLikeProperties;

#[pumpkin_block("minecraft:daylight_detector")]
pub struct DaylightDetectorBlock;

impl BlockBehaviour for DaylightDetectorBlock {
    fn placed<'a>(&'a self, args: PlacedArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            args.world
                .add_block_entity(Arc::new(DaylightDetectorBlockEntity::new(*args.position)));
            if args.world.dimension.has_skylight {
                DaylightDetectorBlockEntity::update_power(args.world, args.position).await;
            }
        })
    }

    fn broken<'a>(&'a self, args: BrokenArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            args.world.remove_block_entity(args.position);
        })
    }

    fn normal_use<'a>(&'a self, args: NormalUseArgs<'a>) -> BlockFuture<'a, BlockActionResult> {
        Box::pin(async {
            let player_abilities = args.player.abilities.lock();
            if !player_abilities.await.allow_modify_world {
                return BlockActionResult::Pass;
            }

            let (block, state) = args.world.get_block_and_state(args.position);
            if block != &Block::DAYLIGHT_DETECTOR {
                return BlockActionResult::Pass;
            }
            let props = DaylightDetectorProperties::from_state_id(state.id, block);

            self.update_inverted(props, args.world, args.position, block)
                .await;

            args.world
                .emit_game_event(*args.position, GameEventKind::BlockChange)
                .await;

            DaylightDetectorBlockEntity::update_power(args.world, args.position).await;

            BlockActionResult::Success
        })
    }

    fn get_weak_redstone_power<'a>(
        &'a self,
        args: GetRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, u8> {
        Box::pin(async move {
            let props = DaylightDetectorProperties::from_state_id(args.state.id, args.block);

            props.power
        })
    }

    fn emits_redstone_power<'a>(
        &'a self,
        _args: EmitsRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, bool> {
        Box::pin(async move { true })
    }

    fn random_tick<'a>(&'a self, args: RandomTickArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            if args.world.dimension.has_skylight {
                DaylightDetectorBlockEntity::update_power(args.world, args.position).await;
            }
        })
    }

    fn on_scheduled_tick<'a>(&'a self, args: OnScheduledTickArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            if args.world.dimension.has_skylight {
                DaylightDetectorBlockEntity::update_power(args.world, args.position).await;
            }
        })
    }
}

impl DaylightDetectorBlock {
    async fn update_inverted(
        &self,
        props: DaylightDetectorProperties,
        world: &Arc<World>,
        block_pos: &BlockPos,
        block: &Block,
    ) {
        let mut props = props;
        props.inverted = !props.inverted;

        let state = props.to_state_id(block);

        world
            .set_block_state(block_pos, state, BlockFlags::NOTIFY_LISTENERS)
            .await;
    }
}
