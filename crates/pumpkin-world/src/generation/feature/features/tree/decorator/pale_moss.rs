use pumpkin_data::{
    Block, BlockState,
    block_properties::{BlockProperties, PaleHangingMossLikeProperties},
};
use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, RandomImpl},
};

use crate::{
    generation::{feature::configured_features::CONFIGURED_FEATURES, proto_chunk::GenerationCache},
    world::WorldPortalExt,
};

#[allow(clippy::struct_field_names)]
pub struct PaleMossTreeDecorator {
    pub leaves_probability: f32,
    pub trunk_probability: f32,
    pub ground_probability: f32,
}

impl PaleMossTreeDecorator {
    #[allow(clippy::too_many_arguments)]
    pub fn generate<T: GenerationCache>(
        &self,
        chunk: &mut T,
        block_registry: &dyn WorldPortalExt,
        min_y: i8,
        height: u16,
        random: &mut RandomGenerator,
        log_positions: &[BlockPos],
        foliage_positions: &[BlockPos],
    ) {
        if log_positions.is_empty() {
            return;
        }

        let mut shuffled_logs = log_positions.to_vec();
        for index in (1..shuffled_logs.len()).rev() {
            let swap = random.next_bounded_i32((index + 1) as i32) as usize;
            shuffled_logs.swap(index, swap);
        }
        let Some(origin) = shuffled_logs.iter().min_by_key(|position| position.0.y) else {
            return;
        };
        let origin = *origin;

        if random.next_f32() < self.ground_probability
            && let Some(feature) = CONFIGURED_FEATURES
                .get(&pumpkin_data::configured_feature::ConfiguredFeature::PaleMossPatch)
        {
            feature.generate(
                chunk,
                block_registry,
                min_y,
                height,
                pumpkin_data::placed_feature::PlacedFeature::PaleMossPatch,
                random,
                origin.up(),
            );
        }

        for pos in log_positions {
            if random.next_f32() < self.trunk_probability {
                let down = pos.down();
                if chunk.is_air(&down.0) {
                    Self::add_moss_hanger(chunk, random, down);
                }
            }
        }
        for pos in foliage_positions {
            if random.next_f32() < self.leaves_probability {
                let down = pos.down();
                if chunk.is_air(&down.0) {
                    Self::add_moss_hanger(chunk, random, down);
                }
            }
        }
    }

    fn add_moss_hanger<T: GenerationCache>(
        chunk: &mut T,
        random: &mut RandomGenerator,
        mut pos: BlockPos,
    ) {
        while chunk.is_air(&pos.down().0) && random.next_f32() >= 0.5 {
            let state =
                PaleHangingMossLikeProperties { tip: false }.to_state_id(&Block::PALE_HANGING_MOSS);
            chunk.set_block_state(&pos.0, BlockState::from_id(state));
            pos = pos.down();
        }
        let state =
            PaleHangingMossLikeProperties { tip: true }.to_state_id(&Block::PALE_HANGING_MOSS);
        chunk.set_block_state(&pos.0, BlockState::from_id(state));
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_data::{
        Block, BlockState,
        block_properties::{BlockProperties, PaleHangingMossLikeProperties},
    };

    #[test]
    fn hanging_moss_tip_property_round_trips() {
        for tip in [false, true] {
            let properties = PaleHangingMossLikeProperties { tip };
            let state = BlockState::from_id(properties.to_state_id(&Block::PALE_HANGING_MOSS));
            assert!(
                PaleHangingMossLikeProperties::from_state_id(state.id, &Block::PALE_HANGING_MOSS)
                    == properties
            );
        }
    }
}
