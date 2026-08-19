use pumpkin_data::{
    Block, BlockState,
    block_properties::{BlockProperties, CocoaLikeProperties, HorizontalFacing},
};
use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, RandomImpl},
};

use crate::generation::proto_chunk::GenerationCache;

pub struct CocoaTreeDecorator {
    pub probability: f32,
}

impl CocoaTreeDecorator {
    pub fn generate<T: GenerationCache>(
        &self,
        chunk: &mut T,
        random: &mut RandomGenerator,
        log_positions: &[BlockPos],
    ) {
        if random.next_f32() >= self.probability {
            return;
        }
        let Some(first_log) = log_positions.first() else {
            return;
        };
        let tree_y = first_log.0.y;

        // Direction.Plane.HORIZONTAL order is NORTH, EAST, SOUTH, WEST in
        // vanilla.  Each direction gets an independent 25% attempt; cocoa's
        // FACING points outward while the block is placed on the opposite
        // side of the trunk.
        for log in log_positions {
            if log.0.y - tree_y > 2 {
                continue;
            }
            for direction in [
                HorizontalFacing::North,
                HorizontalFacing::East,
                HorizontalFacing::South,
                HorizontalFacing::West,
            ] {
                if random.next_f32() > 0.25 {
                    continue;
                }
                let target = log.offset(direction.opposite().to_offset());
                if !chunk.is_air(&target.0) {
                    continue;
                }
                let mut props = CocoaLikeProperties::default(&Block::COCOA);
                props.age = random.next_bounded_i32(3) as u8;
                props.facing = direction;
                chunk.set_block_state(
                    &target.0,
                    BlockState::from_id(props.to_state_id(&Block::COCOA)),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_data::{
        Block, BlockState,
        block_properties::{BlockProperties, CocoaLikeProperties, HorizontalFacing},
    };

    #[test]
    fn cocoa_state_round_trips_age_and_outward_facing() {
        for facing in HorizontalFacing::all() {
            for age in 0..=2 {
                let props = CocoaLikeProperties { age, facing };
                let state = BlockState::from_id(props.to_state_id(&Block::COCOA));
                let decoded = CocoaLikeProperties::from_state_id(state.id, &Block::COCOA);
                assert!(decoded == props);
            }
        }
    }
}
