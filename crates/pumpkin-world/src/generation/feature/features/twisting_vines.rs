use pumpkin_data::{
    Block, BlockState,
    block_properties::{BlockProperties, KelpLikeProperties},
};
use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, RandomImpl},
};

use crate::generation::proto_chunk::GenerationCache;

pub struct TwistingVinesFeature {
    pub spread_width: i32,
    pub spread_height: i32,
    pub max_height: i32,
}

impl TwistingVinesFeature {
    pub fn generate<T: GenerationCache>(
        &self,
        chunk: &mut T,
        _min_y: i8,
        _height: u16,
        _feature_name: pumpkin_data::placed_feature::PlacedFeature,
        random: &mut RandomGenerator,
        pos: BlockPos,
    ) -> bool {
        if Self::is_invalid_location(chunk, &pos) {
            return false;
        }

        let mut placed = false;

        for _ in 0..self.spread_width * self.spread_width {
            // Mth.nextInt(random, -spread, spread) is inclusive at both
            // ends.  A difference of two bounded draws (the old code) never
            // reached either edge and consumed twice as many RNG values.
            let offset_x = random.next_bounded_i32(self.spread_width * 2 + 1) - self.spread_width;
            let offset_y = random.next_bounded_i32(self.spread_height * 2 + 1) - self.spread_height;
            let offset_z = random.next_bounded_i32(self.spread_width * 2 + 1) - self.spread_width;

            let mut mutable_pos = pos.add(offset_x, offset_y, offset_z);

            if self.find_target_y(chunk, &mut mutable_pos)
                && !Self::is_invalid_location(chunk, &mutable_pos)
            {
                let mut height = random.next_bounded_i32(self.max_height) + 1;
                if random.next_bounded_i32(6) == 0 {
                    height *= 2;
                }
                if random.next_bounded_i32(5) == 0 {
                    height = 1;
                }

                Self::generate_column(chunk, random, &mutable_pos, height);
                placed = true;
            }
        }

        placed
    }

    fn generate_column<T: GenerationCache>(
        chunk: &mut T,
        random: &mut RandomGenerator,
        pos: &BlockPos,
        height: i32,
    ) {
        let mut current_pos = *pos;
        for i in 0..height {
            if !GenerationCache::get_block_state(chunk, &current_pos.0)
                .to_state()
                .is_air()
            {
                break;
            }

            if i == height - 1
                || !GenerationCache::get_block_state(chunk, &current_pos.up().0)
                    .to_state()
                    .is_air()
            {
                // The head uses the same AGE property group as kelp and
                // weeping/twisting vines.  Vanilla samples 17..=25 only for
                // the final head; plant sections have no AGE property.
                let mut props = KelpLikeProperties::default(&Block::TWISTING_VINES);
                props.age = 17 + random.next_bounded_i32(9) as u8;
                chunk.set_block_state(
                    &current_pos.0,
                    BlockState::from_id(props.to_state_id(&Block::TWISTING_VINES)),
                );
                break;
            }
            chunk.set_block_state(&current_pos.0, Block::TWISTING_VINES_PLANT.default_state);
            current_pos = current_pos.up();
        }
    }

    fn is_invalid_location<T: GenerationCache>(chunk: &T, pos: &BlockPos) -> bool {
        if !GenerationCache::get_block_state(chunk, &pos.0)
            .to_state()
            .is_air()
        {
            return true;
        }

        let block_below = GenerationCache::get_block_state(chunk, &pos.down().0).to_block_id();
        !matches!(
            block_below,
            id if id == Block::NETHERRACK.id
                || id == Block::WARPED_NYLIUM.id
                || id == Block::WARPED_WART_BLOCK.id
        )
    }

    fn find_target_y<T: GenerationCache>(&self, chunk: &T, pos: &mut BlockPos) -> bool {
        // Try to find a valid floor by looking down
        let mut current = *pos;
        for _ in 0..self.spread_height {
            current = current.down();
            if !GenerationCache::get_block_state(chunk, &current.0)
                .to_state()
                .is_air()
            {
                *pos = current.up();
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_data::{
        Block, BlockState,
        block_properties::{BlockProperties, KelpLikeProperties},
    };

    #[test]
    fn twisting_vines_head_age_round_trips_for_vanilla_range() {
        for age in 17..=25 {
            let mut props = KelpLikeProperties::default(&Block::TWISTING_VINES);
            props.age = age;
            let state = BlockState::from_id(props.to_state_id(&Block::TWISTING_VINES));
            let decoded = KelpLikeProperties::from_state_id(state.id, &Block::TWISTING_VINES);
            assert_eq!(decoded.age, age);
        }
    }

    #[test]
    fn twisting_vines_head_has_all_26_age_states() {
        assert_eq!(Block::TWISTING_VINES.states.len(), 26);
        assert_eq!(Block::TWISTING_VINES_PLANT.states.len(), 1);
    }
}
