use pumpkin_data::{
    Block, BlockDirection, BlockState,
    block_properties::{BlockProperties, KelpLikeProperties},
};
use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, RandomImpl},
};

use crate::generation::proto_chunk::GenerationCache;

/// Vanilla's roof nether-wart + weeping-vines feature.
pub struct WeepingVinesFeature;

#[inline]
fn is_roof_support(block: pumpkin_data::BlockId) -> bool {
    block == Block::NETHERRACK.id || block == Block::NETHER_WART_BLOCK.id
}

impl WeepingVinesFeature {
    #[allow(clippy::unused_self)]
    pub fn generate<T: GenerationCache>(
        &self,
        chunk: &mut T,
        _min_y: i8,
        _height: u16,
        _feature: pumpkin_data::placed_feature::PlacedFeature,
        random: &mut RandomGenerator,
        pos: BlockPos,
    ) -> bool {
        if !chunk.is_air(&pos.0)
            || !is_roof_support(GenerationCache::get_block_state(chunk, &pos.up().0).to_block_id())
        {
            return false;
        }

        Self::place_roof_nether_wart(chunk, random, &pos);
        Self::place_roof_weeping_vines(chunk, random, &pos);
        true
    }

    fn place_roof_nether_wart<T: GenerationCache>(
        chunk: &mut T,
        random: &mut RandomGenerator,
        origin: &BlockPos,
    ) {
        chunk.set_block_state(&origin.0, Block::NETHER_WART_BLOCK.default_state);
        for _ in 0..200 {
            // Java uses nextInt(6) - nextInt(6), and nextInt(2) - nextInt(5)
            // here (not an inclusive symmetric helper).
            let candidate = origin.add(
                random.next_bounded_i32(6) - random.next_bounded_i32(6),
                random.next_bounded_i32(2) - random.next_bounded_i32(5),
                random.next_bounded_i32(6) - random.next_bounded_i32(6),
            );
            if !chunk.is_air(&candidate.0) {
                continue;
            }
            let neighbours = BlockDirection::all()
                .iter()
                .filter(|direction| {
                    is_roof_support(
                        GenerationCache::get_block_state(
                            chunk,
                            &candidate.offset(direction.to_offset()).0,
                        )
                        .to_block_id(),
                    )
                })
                .take(2)
                .count();
            if neighbours == 1 {
                chunk.set_block_state(&candidate.0, Block::NETHER_WART_BLOCK.default_state);
            }
        }
    }

    fn place_roof_weeping_vines<T: GenerationCache>(
        chunk: &mut T,
        random: &mut RandomGenerator,
        origin: &BlockPos,
    ) {
        for _ in 0..100 {
            let candidate = origin.add(
                random.next_bounded_i32(8) - random.next_bounded_i32(8),
                random.next_bounded_i32(2) - random.next_bounded_i32(7),
                random.next_bounded_i32(8) - random.next_bounded_i32(8),
            );
            if !chunk.is_air(&candidate.0)
                || !is_roof_support(
                    GenerationCache::get_block_state(chunk, &candidate.up().0).to_block_id(),
                )
            {
                continue;
            }

            let mut height = random.next_bounded_i32(8) + 1;
            if random.next_bounded_i32(6) == 0 {
                height *= 2;
            }
            if random.next_bounded_i32(5) == 0 {
                height = 1;
            }
            Self::place_weeping_vines_column(chunk, random, candidate, height);
        }
    }

    fn place_weeping_vines_column<T: GenerationCache>(
        chunk: &mut T,
        random: &mut RandomGenerator,
        mut pos: BlockPos,
        total_height: i32,
    ) {
        for height in 0..=total_height {
            if chunk.is_air(&pos.0) {
                if height == total_height || !chunk.is_air(&pos.down().0) {
                    let mut props = KelpLikeProperties::default(&Block::WEEPING_VINES);
                    props.age = 17 + random.next_bounded_i32(9) as u8;
                    chunk.set_block_state(
                        &pos.0,
                        BlockState::from_id(props.to_state_id(&Block::WEEPING_VINES)),
                    );
                    return;
                }
                chunk.set_block_state(&pos.0, Block::WEEPING_VINES_PLANT.default_state);
            }
            pos = pos.down();
        }
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_data::{
        Block, BlockState,
        block_properties::{BlockProperties, KelpLikeProperties},
    };

    use super::is_roof_support;

    #[test]
    fn weeping_vines_support_matches_vanilla_roof_blocks() {
        assert!(is_roof_support(Block::NETHERRACK.id));
        assert!(is_roof_support(Block::NETHER_WART_BLOCK.id));
        assert!(!is_roof_support(Block::WARPED_WART_BLOCK.id));
    }

    #[test]
    fn weeping_vines_head_age_round_trips_for_vanilla_range() {
        for age in 17..=25 {
            let mut props = KelpLikeProperties::default(&Block::WEEPING_VINES);
            props.age = age;
            let state = BlockState::from_id(props.to_state_id(&Block::WEEPING_VINES));
            assert_eq!(
                KelpLikeProperties::from_state_id(state.id, &Block::WEEPING_VINES).age,
                age
            );
        }
    }
}
