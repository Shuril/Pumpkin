use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::vector3::Vector3;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use crate::block::{BlockBehaviour, BlockFuture, OnNeighborUpdateArgs, PlacedArgs};
use pumpkin_data::block_properties::{
    BlockProperties, DoubleBlockHalf, TallSeagrassLikeProperties,
};
use pumpkin_data::dimension::Dimension;
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{Block, BlockId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

#[pumpkin_block("minecraft:sponge")]
pub struct SpongeBlock;

impl SpongeBlock {
    const MAX_DEPTH: i32 = 6;
    const MAX_COUNT: usize = 64;

    /// Blocks that vanilla's sponge traversal can remove.  The fluid check is
    /// deliberately kept separate: kelp and seagrass carry water in vanilla's
    /// `FluidState`, while Pumpkin represents their water occupancy by the
    /// aquatic block itself.
    #[must_use]
    fn is_absorbable_water_block(block: &Block) -> bool {
        matches!(
            block.id,
            BlockId::WATER
                | BlockId::BUBBLE_COLUMN
                | BlockId::KELP
                | BlockId::KELP_PLANT
                | BlockId::SEAGRASS
                | BlockId::TALL_SEAGRASS
        )
    }

    async fn remove_absorbable_block(
        world: &Arc<crate::world::World>,
        position: &BlockPos,
        block: &Block,
    ) {
        let state = world.get_block_state(position);
        world
            .break_block(position, None, BlockFlags::NOTIFY_ALL)
            .await;

        // Tall seagrass is one logical plant represented by two block states.
        // The normal player-break path removes the opposite half through the
        // block callback; sponge absorption has no player callback, so mirror
        // that cleanup explicitly without generating a second drop.
        if block.id == Block::TALL_SEAGRASS.id {
            let props = TallSeagrassLikeProperties::from_state_id(state.id, block);
            let other_position = match props.half {
                DoubleBlockHalf::Lower => position.up(),
                DoubleBlockHalf::Upper => position.down(),
            };
            let (other_block, _) = world.get_block_and_state(&other_position);
            if other_block.id == Block::TALL_SEAGRASS.id {
                world
                    .set_block_state(
                        &other_position,
                        Block::AIR.default_state.id,
                        BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
                    )
                    .await;
            }
        }
    }

    pub async fn absorb_water(world: &Arc<crate::world::World>, position: &BlockPos) -> bool {
        let mut water_blocks = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        // Start from the sponge position
        queue.push_back(*position);
        visited.insert(*position);

        while let Some(current_pos) = queue.pop_front() {
            for direction in &[
                (1, 0, 0),
                (-1, 0, 0),
                (0, 1, 0),
                (0, -1, 0),
                (0, 0, 1),
                (0, 0, -1),
            ] {
                let next_pos = BlockPos::new(
                    current_pos.0.x + direction.0,
                    current_pos.0.y + direction.1,
                    current_pos.0.z + direction.2,
                );

                if visited.contains(&next_pos) {
                    continue;
                }

                let taxicab_dist = (next_pos.0.x - position.0.x).abs()
                    + (next_pos.0.y - position.0.y).abs()
                    + (next_pos.0.z - position.0.z).abs();

                // SpongeBlock.removeWaterBreadthFirstSearch uses a maximum
                // traversal depth of six and accepts at most 64 nodes.
                if taxicab_dist > Self::MAX_DEPTH || water_blocks.len() >= Self::MAX_COUNT {
                    continue;
                }

                visited.insert(next_pos);
                let (block, _state) = world.get_block_and_state(&next_pos);

                // Only traverse water and water-bearing plants.  This keeps
                // the search from jumping through air/solid blocks while still
                // reaching kelp and seagrass chains as vanilla does.
                if Self::is_absorbable_water_block(block) {
                    water_blocks.push(next_pos);
                    queue.push_back(next_pos);
                }
            }
        }

        if water_blocks.is_empty() {
            false
        } else {
            for water_pos in &water_blocks {
                // Use the normal break path so kelp/seagrass drops, double
                // plant cleanup and block events follow the same code as a
                // vanilla bucket pickup.  The sponge itself is changed only
                // after every accepted node has been processed.
                let (block, _) = world.get_block_and_state(water_pos);
                // A tall plant can invalidate a second queued half when its
                // first half is removed.  Do not turn that replacement into a
                // fresh water-removal operation.
                if !Self::is_absorbable_water_block(block) {
                    continue;
                }
                Self::remove_absorbable_block(world, water_pos, block).await;
            }
            world
                .set_block_state(
                    position,
                    Block::WET_SPONGE.default_state.id,
                    BlockFlags::NOTIFY_ALL,
                )
                .await;

            world.play_block_sound(Sound::BlockSpongeAbsorb, SoundCategory::Blocks, *position);

            true
        }
    }
}

impl BlockBehaviour for SpongeBlock {
    fn placed<'a>(&'a self, args: PlacedArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            // Attempt to absorb water on placement
            Self::absorb_water(args.world, args.position).await;
        })
    }

    fn on_neighbor_update<'a>(&'a self, args: OnNeighborUpdateArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            // Vanilla retries on every neighbor update: a plant, fluid, or
            // waterlogged state may have changed even when the source block is
            // not literally the water block.
            Self::absorb_water(args.world, args.position).await;
        })
    }
}

#[pumpkin_block("minecraft:wet_sponge")]
pub struct WetSpongeBlock;

impl BlockBehaviour for WetSpongeBlock {
    fn placed<'a>(&'a self, args: PlacedArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            // Check if placed in Nether, if so, dry out
            if args.world.dimension == Dimension::THE_NETHER {
                args.world
                    .set_block_state(
                        args.position,
                        Block::SPONGE.default_state.id,
                        BlockFlags::NOTIFY_ALL,
                    )
                    .await;

                // Play dry sound and spawn smoke particles
                args.world.play_block_sound(
                    Sound::BlockWetSpongeDries,
                    SoundCategory::Blocks,
                    *args.position,
                );

                args.world.spawn_particle(
                    Vector3::new(
                        args.position.0.x as f64 + 0.5,
                        args.position.0.y as f64 + 1.0,
                        args.position.0.z as f64 + 0.5,
                    ),
                    Vector3::new(0.25, 0.0, 0.25),
                    0.01,
                    16,
                    Particle::Cloud,
                );
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::SpongeBlock;
    use pumpkin_data::{Block, BlockId};

    #[test]
    fn sponge_absorbs_vanilla_water_and_aquatic_plants() {
        for block in [
            &Block::WATER,
            &Block::BUBBLE_COLUMN,
            &Block::KELP,
            &Block::KELP_PLANT,
            &Block::SEAGRASS,
            &Block::TALL_SEAGRASS,
        ] {
            assert!(SpongeBlock::is_absorbable_water_block(block));
        }
    }

    #[test]
    fn sponge_does_not_traverse_air_or_solid_blocks() {
        assert!(!SpongeBlock::is_absorbable_water_block(&Block::AIR));
        assert!(!SpongeBlock::is_absorbable_water_block(&Block::STONE));
        assert_ne!(Block::STONE.id, BlockId::WATER);
    }
}
