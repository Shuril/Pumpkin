use std::collections::HashSet;

use pumpkin_data::BlockDirection;
use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, RandomImpl},
};

use crate::{
    generation::{block_state_provider::BlockStateProvider, proto_chunk::GenerationCache},
    world::WorldPortalExt,
};

pub struct AttachedToLeavesTreeDecorator {
    pub probability: f32,
    pub exclusion_radius_xz: i32,
    pub exclusion_radius_y: i32,
    pub block_provider: BlockStateProvider,
    pub required_empty_blocks: i32,
    pub directions: Vec<BlockDirection>,
}

impl AttachedToLeavesTreeDecorator {
    pub fn generate<T: GenerationCache>(
        &self,
        chunk: &mut T,
        block_registry: &dyn WorldPortalExt,
        random: &mut RandomGenerator,
        foliage_positions: &[BlockPos],
    ) {
        if self.directions.is_empty() || foliage_positions.is_empty() {
            return;
        }

        // Vanilla consumes RNG from a shuffled copy of the foliage positions,
        // then draws a direction and probability for every candidate.  The
        // blacklist is a cuboid around each successful placement and prevents
        // overlapping propagules/moss from being placed by the same decorator.
        let mut foliage = foliage_positions.to_vec();
        for index in (1..foliage.len()).rev() {
            let swap = random.next_bounded_i32((index + 1) as i32) as usize;
            foliage.swap(index, swap);
        }
        let mut blacklist = HashSet::new();

        for leaf_pos in foliage {
            let direction =
                self.directions[random.next_bounded_i32(self.directions.len() as i32) as usize];
            let placement = leaf_pos.offset(direction.to_offset());
            if blacklist.contains(&placement)
                || random.next_f32() >= self.probability
                || !has_required_empty_blocks(
                    chunk,
                    leaf_pos,
                    direction,
                    self.required_empty_blocks,
                )
            {
                continue;
            }

            let corner1 = placement.add(
                -self.exclusion_radius_xz,
                -self.exclusion_radius_y,
                -self.exclusion_radius_xz,
            );
            let corner2 = placement.add(
                self.exclusion_radius_xz,
                self.exclusion_radius_y,
                self.exclusion_radius_xz,
            );
            for blocked in BlockPos::iterate(corner1, corner2) {
                blacklist.insert(blocked);
            }

            if GenerationCache::get_block_state(chunk, &placement.0)
                .to_state()
                .is_air()
            {
                chunk.set_block_state(
                    &placement.0,
                    self.block_provider
                        .get(random, placement, chunk, block_registry),
                );
            }
        }
    }
}

fn has_required_empty_blocks<T: GenerationCache>(
    chunk: &T,
    leaf_pos: BlockPos,
    direction: BlockDirection,
    required: i32,
) -> bool {
    (1..=required).all(|distance| {
        let position = leaf_pos.offset_dir(direction.to_offset(), distance);
        chunk.is_air(&position.0)
    })
}

#[cfg(test)]
mod tests {
    use pumpkin_data::BlockDirection;
    use pumpkin_util::math::position::BlockPos;

    #[test]
    fn required_empty_range_is_directional_and_one_based() {
        let origin = BlockPos::new(10, 64, 10);
        let east = origin.offset_dir(BlockDirection::East.to_offset(), 2);
        assert_eq!(east.0.x, 12);
        assert_eq!(east.0.y, 64);
        assert_eq!(east.0.z, 10);
    }

    #[test]
    fn exclusion_box_is_inclusive() {
        let center = BlockPos::new(0, 0, 0);
        let positions: Vec<_> =
            BlockPos::iterate(center.add(-1, 0, -1), center.add(1, 0, 1)).collect();
        assert_eq!(positions.len(), 9);
        assert!(positions.contains(&center));
    }
}
