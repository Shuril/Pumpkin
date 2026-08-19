use pumpkin_data::{
    Block, BlockDirection, BlockState,
    block_properties::{Axis, BlockProperties, CreakingHeartLikeProperties, CreakingHeartState},
    tag,
};
use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, RandomImpl},
};

use crate::generation::proto_chunk::GenerationCache;

/// Places the natural dormant Creaking Heart used by pale-oak trees.
///
/// Vanilla shuffles the generated log positions, then accepts the first log
/// whose six neighbours are all in `#minecraft:logs`.  The order and random
/// draws are important: this decorator is seed-sensitive and must not choose
/// a fixed trunk position.
pub struct CreakingHeartTreeDecorator {
    pub probability: f32,
}

fn shuffled_positions(positions: &[BlockPos], random: &mut RandomGenerator) -> Vec<BlockPos> {
    let mut shuffled = positions.to_vec();
    for index in (1..shuffled.len()).rev() {
        let swap = random.next_bounded_i32((index + 1) as i32) as usize;
        shuffled.swap(index, swap);
    }
    shuffled
}

fn is_log<T: GenerationCache>(chunk: &T, pos: &BlockPos) -> bool {
    GenerationCache::get_block_state(chunk, &pos.0)
        .to_block_id()
        .has_tag(tag::Block::MINECRAFT_LOGS)
}

impl CreakingHeartTreeDecorator {
    pub fn generate<T: GenerationCache>(
        &self,
        chunk: &mut T,
        random: &mut RandomGenerator,
        log_positions: &[BlockPos],
    ) {
        if log_positions.is_empty() || random.next_f32() >= self.probability {
            return;
        }

        let Some(target) = shuffled_positions(log_positions, random)
            .into_iter()
            .find(|pos| {
                BlockDirection::all()
                    .iter()
                    .all(|direction| is_log(chunk, &pos.offset(direction.to_offset())))
            })
        else {
            return;
        };

        let properties = CreakingHeartLikeProperties {
            r#axis: Axis::Y,
            r#creaking_heart_state: CreakingHeartState::Dormant,
            r#natural: true,
        };
        chunk.set_block_state(
            &target.0,
            BlockState::from_id(properties.to_state_id(&Block::CREAKING_HEART)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creaking_heart_uses_natural_dormant_vertical_state() {
        let properties = CreakingHeartLikeProperties {
            r#axis: Axis::Y,
            r#creaking_heart_state: CreakingHeartState::Dormant,
            r#natural: true,
        };
        let state = BlockState::from_id(properties.to_state_id(&Block::CREAKING_HEART));
        assert!(
            CreakingHeartLikeProperties::from_state_id(state.id, &Block::CREAKING_HEART)
                == properties
        );
    }

    #[test]
    fn only_log_tagged_states_are_valid_neighbours() {
        assert!(Block::OAK_LOG.id.has_tag(tag::Block::MINECRAFT_LOGS));
        assert!(Block::PALE_OAK_LOG.id.has_tag(tag::Block::MINECRAFT_LOGS));
        assert!(!Block::OAK_LEAVES.id.has_tag(tag::Block::MINECRAFT_LOGS));
    }
}
