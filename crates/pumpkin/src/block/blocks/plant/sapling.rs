use pumpkin_data::BlockStateId;
use pumpkin_data::block_properties::BlockProperties;
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use rand::RngExt;
use std::sync::Arc;

use crate::block::blocks::plant::PlantBlockBase;
use crate::block::{
    BlockBehaviour, BlockFuture, CanPlaceAtArgs, GetStateForNeighborUpdateArgs, RandomTickArgs,
};
use crate::world::World;

type SaplingProperties = pumpkin_data::block_properties::OakSaplingLikeProperties;

#[inline]
const fn sapling_growth_roll_passes(roll: u8) -> bool {
    roll == 0
}

#[pumpkin_block_from_tag("minecraft:saplings")]
pub struct SaplingBlock;

impl SaplingBlock {
    async fn generate(&self, world: &Arc<World>, pos: &BlockPos) {
        let (block, state) = world.get_block_and_state_id(pos);
        let mut props = SaplingProperties::from_state_id(state, block);
        if props.stage == 0 {
            props.stage = 1;
            world
                .set_block_state(pos, props.to_state_id(block), BlockFlags::NOTIFY_ALL)
                .await;
        } else {
            //TODO generate tree
        }
    }
}

impl BlockBehaviour for SaplingBlock {
    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        <Self as PlantBlockBase>::can_place_at(self, args.block_accessor, args.position)
    }

    fn get_state_for_neighbor_update<'a>(
        &'a self,
        args: GetStateForNeighborUpdateArgs<'a>,
    ) -> BlockFuture<'a, BlockStateId> {
        Box::pin(async move {
            <Self as PlantBlockBase>::get_state_for_neighbor_update(
                self,
                args.world,
                args.position,
                args.state_id,
            )
            .await
        })
    }

    fn random_tick<'a>(&'a self, args: RandomTickArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            // SaplingBlock.randomTick advances the stage only on one of seven
            // random ticks.  Tree generation itself remains a separate
            // configured-feature task; without this gate saplings reached
            // stage two on their very first scheduled tick.
            if !sapling_growth_roll_passes(rand::rng().random_range(0..7)) {
                return;
            }
            self.generate(args.world, args.position).await;
        })
    }
}

impl PlantBlockBase for SaplingBlock {}

#[cfg(test)]
mod tests {
    use super::sapling_growth_roll_passes;

    #[test]
    fn sapling_stage_gate_is_one_in_seven() {
        assert!(sapling_growth_roll_passes(0));
        for roll in 1..7 {
            assert!(!sapling_growth_roll_passes(roll));
        }
    }
}
