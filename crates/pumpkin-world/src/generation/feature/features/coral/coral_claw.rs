use crate::{generation::proto_chunk::GenerationCache, world::WorldPortalExt};
use pumpkin_data::{BlockDirection, HorizontalFacingExt, block_properties::HorizontalFacing, tag};
use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, RandomImpl},
};

use super::CoralFeature;

pub struct CoralClawFeature;

fn shuffled_claw_directions(
    direction: HorizontalFacing,
    random: &mut RandomGenerator,
) -> [HorizontalFacing; 3] {
    let mut directions = [
        direction,
        direction.rotate_clockwise(),
        direction.rotate_counter_clockwise(),
    ];
    for index in (1..directions.len()).rev() {
        let swap = random.next_bounded_i32(index as i32 + 1) as usize;
        directions.swap(index, swap);
    }
    directions
}

impl CoralClawFeature {
    #[allow(clippy::too_many_arguments)]
    pub fn generate<T: GenerationCache>(
        chunk: &mut T,
        block_registry: &dyn WorldPortalExt,
        _min_y: i8,
        _height: u16,
        _feature: pumpkin_data::placed_feature::PlacedFeature, // This placed feature
        random: &mut RandomGenerator,
        pos: BlockPos,
    ) -> bool {
        // First lets get a random coral
        let block = CoralFeature::get_random_tag_entry(tag::Block::MINECRAFT_CORAL_BLOCKS, random);
        if !CoralFeature::generate_coral_piece(chunk, block_registry, random, block, pos) {
            return false;
        }
        let direction = BlockDirection::random_horizontal(random);
        let i = random.next_bounded_i32(2) + 2;
        // Vanilla shuffles exactly [direction, clockwise, counter-clockwise] and then
        // takes the first i entries. Do not replace this with the four world directions:
        // the opposite side is intentionally never visited and the Fisher–Yates draws are
        // part of the feature's seed-visible random stream.
        let directions = shuffled_claw_directions(direction, random)
            .into_iter()
            .take(i as usize);
        'block0: for direction2 in directions {
            let mut pos = pos;
            let j = random.next_bounded_i32(2) + 1;
            pos = pos.offset(direction2.to_offset());

            let branch_direction;

            let k = if direction2 == direction {
                branch_direction = direction.to_block_direction();
                random.next_bounded_i32(3) + 2
            } else {
                pos = pos.up();
                branch_direction = if random.next_bounded_i32(2) == 0 {
                    direction2.to_block_direction()
                } else {
                    BlockDirection::Up
                };
                random.next_bounded_i32(3) + 3
            };

            for _ in 0..j {
                if !CoralFeature::generate_coral_piece(chunk, block_registry, random, block, pos) {
                    break;
                }
                pos = pos.offset(branch_direction.to_offset());
            }

            pos = pos.offset(branch_direction.to_offset());
            pos = pos.up();

            for _l in 0..k {
                pos = pos.offset(direction.opposite().to_offset());
                if !CoralFeature::generate_coral_piece(chunk, block_registry, random, block, pos) {
                    continue 'block0;
                }
                if random.next_f32() < 0.25 {
                    pos = pos.up();
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::shuffled_claw_directions;
    use pumpkin_data::HorizontalFacingExt;
    use pumpkin_data::block_properties::HorizontalFacing;
    use pumpkin_util::random::{RandomGenerator, RandomImpl, legacy_rand::LegacyRand};

    #[test]
    fn claw_shuffle_uses_only_forward_and_side_directions() {
        let direction = HorizontalFacing::North;
        let mut random = RandomGenerator::Legacy(LegacyRand::from_seed(0x5eed));
        let directions = shuffled_claw_directions(direction, &mut random);
        assert_eq!(directions.len(), 3);
        assert!(directions.contains(&direction));
        assert!(directions.contains(&direction.rotate_clockwise()));
        assert!(directions.contains(&direction.rotate_counter_clockwise()));
        assert!(!directions.iter().any(|candidate| {
            candidate.to_block_direction() == direction.to_block_direction().opposite()
        }));
        // The helper must consume the two Fisher–Yates draws, not just permute a copy.
        assert_ne!(random.next_bounded_i32(4), 0);
    }
}
