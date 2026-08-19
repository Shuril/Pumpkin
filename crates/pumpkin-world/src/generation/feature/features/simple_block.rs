use pumpkin_data::{
    Block, BlockDirection,
    block_properties::{BlockProperties, DoubleBlockHalf, TallSeagrassLikeProperties},
};
use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, RandomImpl},
};

use crate::generation::proto_chunk::GenerationCache;
use crate::tick::TickPriority;
use crate::{
    generation::block_state_provider::BlockStateProvider,
    world::{BlockAccessor, WorldPortalExt},
};

pub struct SimpleBlockFeature {
    pub to_place: BlockStateProvider,
    pub schedule_tick: Option<bool>,
}

impl SimpleBlockFeature {
    pub fn generate<T: GenerationCache>(
        &self,
        block_registry: &dyn WorldPortalExt,
        chunk: &mut T,
        random: &mut RandomGenerator,
        pos: BlockPos,
    ) -> bool {
        let state = self.to_place.get(random, pos, chunk, block_registry);
        let block = Block::from_state_id(state.id);
        let block_accessor: &dyn BlockAccessor = chunk;
        if !block_registry.can_place_at(block, state, block_accessor, &pos) {
            return false;
        }

        // `SimpleBlockFeature` delegates these two block families to their
        // vanilla placement helpers rather than writing only the configured
        // lower state.  A double plant occupies two cells and must be rejected
        // atomically when the upper cell is occupied.
        if TallSeagrassLikeProperties::handles_block_id(block.id) {
            if !chunk.is_air(&pos.up().0) {
                return false;
            }
            let mut lower = TallSeagrassLikeProperties::from_state_id(state.id, block);
            lower.half = DoubleBlockHalf::Lower;
            let mut upper = lower;
            upper.half = DoubleBlockHalf::Upper;
            chunk.set_block_state(
                &pos.0,
                pumpkin_data::BlockState::from_id(lower.to_state_id(block)),
            );
            chunk.set_block_state(
                &pos.up().0,
                pumpkin_data::BlockState::from_id(upper.to_state_id(block)),
            );
        } else if pumpkin_data::block_properties::PaleMossCarpetLikeProperties::handles_block_id(
            block.id,
        ) {
            // Vanilla's MossyCarpetBlock.placeAt is more than a simple block
            // write: it derives the four wall sides from neighbouring support
            // faces, then may place a second, non-base carpet layer above it.
            // The second layer is deliberately decided from the feature RNG,
            // not a process-global random source, so chunk generation remains
            // reproducible.
            let base_state = pale_moss_state(chunk, pos, true);
            chunk.set_block_state(&pos.0, base_state);

            let above = pos.up();
            let above_previous = GenerationCache::get_block_state(chunk, &above.0).to_state();
            if above_previous.replaceable() {
                let mut topper = pale_moss_state(chunk, above, false);
                let mut props =
                    pumpkin_data::block_properties::PaleMossCarpetLikeProperties::from_state_id(
                        topper.id,
                        &Block::PALE_MOSS_CARPET,
                    );
                if !matches!(
                    props.r#north,
                    pumpkin_data::block_properties::NorthWall::None
                ) && !random.next_bool()
                {
                    props.r#north = pumpkin_data::block_properties::NorthWall::None;
                }
                if !matches!(props.r#east, pumpkin_data::block_properties::EastWall::None)
                    && !random.next_bool()
                {
                    props.r#east = pumpkin_data::block_properties::EastWall::None;
                }
                if !matches!(
                    props.r#south,
                    pumpkin_data::block_properties::SouthWall::None
                ) && !random.next_bool()
                {
                    props.r#south = pumpkin_data::block_properties::SouthWall::None;
                }
                if !matches!(props.r#west, pumpkin_data::block_properties::WestWall::None)
                    && !random.next_bool()
                {
                    props.r#west = pumpkin_data::block_properties::WestWall::None;
                }
                props.r#bottom = false;
                topper =
                    pumpkin_data::BlockState::from_id(props.to_state_id(&Block::PALE_MOSS_CARPET));
                if pale_moss_has_faces(props) {
                    chunk.set_block_state(&above.0, topper);
                    // The lower layer is re-evaluated after the topper exists;
                    // supported sides become TALL exactly as in
                    // MossyCarpetBlock.placeAt.
                    let updated_base = pale_moss_state(chunk, pos, true);
                    chunk.set_block_state(&pos.0, updated_base);
                }
            }
        } else {
            chunk.set_block_state(&pos.0, state);
        }
        if self.schedule_tick.unwrap_or(false) {
            chunk.schedule_block_tick(&pos.0, block, 1, TickPriority::Normal);
        }
        true
    }
}

fn pale_moss_supports<T: GenerationCache>(
    chunk: &T,
    pos: BlockPos,
    direction: BlockDirection,
) -> bool {
    let neighbour = pos.offset(direction.to_offset());
    GenerationCache::get_block_state(chunk, &neighbour.0)
        .to_state()
        .is_side_solid(direction.opposite())
}

fn pale_moss_state<T: GenerationCache>(
    chunk: &T,
    pos: BlockPos,
    base: bool,
) -> &'static pumpkin_data::BlockState {
    use pumpkin_data::block_properties::{
        EastWall, NorthWall, PaleMossCarpetLikeProperties, SouthWall, WestWall,
    };

    let mut props = PaleMossCarpetLikeProperties::default(&Block::PALE_MOSS_CARPET);
    props.r#bottom = base;
    props.r#north = if pale_moss_supports(chunk, pos, BlockDirection::North) {
        NorthWall::Low
    } else {
        NorthWall::None
    };
    props.r#east = if pale_moss_supports(chunk, pos, BlockDirection::East) {
        EastWall::Low
    } else {
        EastWall::None
    };
    props.r#south = if pale_moss_supports(chunk, pos, BlockDirection::South) {
        SouthWall::Low
    } else {
        SouthWall::None
    };
    props.r#west = if pale_moss_supports(chunk, pos, BlockDirection::West) {
        WestWall::Low
    } else {
        WestWall::None
    };

    // When a non-base topper has just been placed above this layer, vanilla
    // raises any shared wall from LOW to TALL.  The check is intentionally
    // based on the topper's actual side state, not merely its block id.
    if base {
        let above = pos.up();
        let above_state = GenerationCache::get_block_state(chunk, &above.0).to_state();
        if above_state.id.to_block() == &Block::PALE_MOSS_CARPET {
            let above_props = PaleMossCarpetLikeProperties::from_state_id(
                above_state.id,
                &Block::PALE_MOSS_CARPET,
            );
            if !above_props.r#bottom
                && props.r#north == NorthWall::Low
                && above_props.r#north != NorthWall::None
            {
                props.r#north = NorthWall::Tall;
            }
            if !above_props.r#bottom
                && props.r#east == EastWall::Low
                && above_props.r#east != EastWall::None
            {
                props.r#east = EastWall::Tall;
            }
            if !above_props.r#bottom
                && props.r#south == SouthWall::Low
                && above_props.r#south != SouthWall::None
            {
                props.r#south = SouthWall::Tall;
            }
            if !above_props.r#bottom
                && props.r#west == WestWall::Low
                && above_props.r#west != WestWall::None
            {
                props.r#west = WestWall::Tall;
            }
        }
    }

    // A topper cannot create a side that its lower carpet does not expose.
    if !base {
        let below = pos.down();
        let below_state = GenerationCache::get_block_state(chunk, &below.0).to_state();
        if below_state.id.to_block() == &Block::PALE_MOSS_CARPET {
            let below_props = PaleMossCarpetLikeProperties::from_state_id(
                below_state.id,
                &Block::PALE_MOSS_CARPET,
            );
            if below_props.r#north == NorthWall::None {
                props.r#north = NorthWall::None;
            }
            if below_props.r#east == EastWall::None {
                props.r#east = EastWall::None;
            }
            if below_props.r#south == SouthWall::None {
                props.r#south = SouthWall::None;
            }
            if below_props.r#west == WestWall::None {
                props.r#west = WestWall::None;
            }
        }
    }

    pumpkin_data::BlockState::from_id(props.to_state_id(&Block::PALE_MOSS_CARPET))
}

fn pale_moss_has_faces(
    props: pumpkin_data::block_properties::PaleMossCarpetLikeProperties,
) -> bool {
    props.r#bottom
        || props.r#north != pumpkin_data::block_properties::NorthWall::None
        || props.r#east != pumpkin_data::block_properties::EastWall::None
        || props.r#south != pumpkin_data::block_properties::SouthWall::None
        || props.r#west != pumpkin_data::block_properties::WestWall::None
}

#[cfg(test)]
mod tests {
    use pumpkin_data::Block;
    use pumpkin_data::block_properties::{
        BlockProperties, NorthWall, PaleMossCarpetLikeProperties,
    };

    use super::pale_moss_has_faces;

    #[test]
    fn pale_moss_topper_requires_a_surviving_wall_face() {
        let mut topper = PaleMossCarpetLikeProperties::default(&Block::PALE_MOSS_CARPET);
        topper.r#bottom = false;
        assert!(!pale_moss_has_faces(topper));

        topper.r#north = NorthWall::Low;
        assert!(pale_moss_has_faces(topper));
    }
}
