use pumpkin_data::BlockDirection;
use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, RandomImpl},
};

use crate::generation::proto_chunk::GenerationCache;
use crate::{generation::block_state_provider::BlockStateProvider, world::WorldPortalExt};

pub struct AttachedToLogsTreeDecorator {
    pub probability: f32,
    pub block_provider: BlockStateProvider,
    pub directions: Vec<BlockDirection>,
}

/// Returns a Fisher–Yates shuffled copy of the log list.
///
/// `AttachedToLogsDecorator` in vanilla deliberately shuffles the input
/// positions before it consumes the per-position random direction and
/// probability draws.  Keeping this as a small pure helper makes the order of
/// RNG consumption explicit and gives us a deterministic unit-test seam.
fn shuffled_log_positions(
    log_positions: &[BlockPos],
    random: &mut RandomGenerator,
) -> Vec<BlockPos> {
    let mut shuffled = log_positions.to_vec();
    for index in (1..shuffled.len()).rev() {
        let swap = random.next_bounded_i32((index + 1) as i32) as usize;
        shuffled.swap(index, swap);
    }
    shuffled
}

#[inline]
fn choose_direction(directions: &[BlockDirection], random: &mut RandomGenerator) -> BlockDirection {
    directions[random.next_bounded_i32(directions.len() as i32) as usize]
}

impl AttachedToLogsTreeDecorator {
    pub fn generate<T: GenerationCache>(
        &self,
        chunk: &mut T,
        block_registry: &dyn WorldPortalExt,
        random: &mut RandomGenerator,
        log_positions: &[BlockPos],
    ) {
        if self.directions.is_empty() {
            return;
        }

        // Vanilla iterates a shuffled copy of the log positions and chooses a
        // fresh random direction for every log.  Reusing directions[0] makes
        // multi-direction decorators seed-incompatible and visibly biases
        // attached blocks toward one side of every tree.
        let shuffled_logs = shuffled_log_positions(log_positions, random);

        for pos in shuffled_logs {
            let direction = choose_direction(&self.directions, random);
            let pos = pos.offset(direction.to_offset());
            if random.next_f32() > self.probability
                || !GenerationCache::get_block_state(chunk, &pos.0)
                    .to_state()
                    .is_air()
            {
                continue;
            }
            chunk.set_block_state(
                &pos.0,
                self.block_provider.get(random, pos, chunk, block_registry),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{choose_direction, shuffled_log_positions};
    use pumpkin_data::BlockDirection;
    use pumpkin_util::{
        math::position::BlockPos,
        random::{RandomGenerator, xoroshiro128::Xoroshiro},
    };

    #[test]
    fn attached_log_order_is_seeded_permutation() {
        let logs: Vec<_> = (0..8).map(|x| BlockPos::new(x, 64, -x)).collect();
        let mut first = RandomGenerator::Xoroshiro(Xoroshiro::from_seed(0xA77AC11));
        let mut second = RandomGenerator::Xoroshiro(Xoroshiro::from_seed(0xA77AC11));
        let shuffled = shuffled_log_positions(&logs, &mut first);
        assert_eq!(shuffled, shuffled_log_positions(&logs, &mut second));

        let mut sorted_original = logs.clone();
        let mut sorted_shuffled = shuffled;
        sorted_original.sort_unstable_by_key(|pos| (pos.0.x, pos.0.y, pos.0.z));
        sorted_shuffled.sort_unstable_by_key(|pos| (pos.0.x, pos.0.y, pos.0.z));
        assert_eq!(sorted_shuffled, sorted_original);
    }

    #[test]
    fn attached_log_direction_selection_uses_declared_set() {
        let directions = [BlockDirection::North, BlockDirection::Up];
        let mut random = RandomGenerator::Xoroshiro(Xoroshiro::from_seed(0xD1CE_0710));
        for _ in 0..64 {
            assert!(directions.contains(&choose_direction(&directions, &mut random)));
        }
    }
}
