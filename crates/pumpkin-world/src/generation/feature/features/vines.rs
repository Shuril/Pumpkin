use pumpkin_data::{Block, BlockDirection, BlockState, block_properties::BlockProperties};
use pumpkin_util::{math::position::BlockPos, random::RandomGenerator};

use crate::generation::proto_chunk::GenerationCache;
use crate::world::WorldPortalExt;

pub struct VinesFeature;

#[inline]
const fn is_acceptable_neighbour(
    state: &pumpkin_data::BlockState,
    direction_to_neighbour: BlockDirection,
) -> bool {
    // MultifaceBlock.canAttachTo checks a full support/collision face on the
    // neighbour's side facing the vine.  The generated side flags are the
    // same constant-time representation and are less strict than requiring a
    // full cube (slabs and other full-face shapes remain valid supports).
    state.is_side_solid(direction_to_neighbour.opposite())
}

impl VinesFeature {
    pub fn generate<T: GenerationCache>(
        chunk: &mut T,
        _block_registry: &dyn WorldPortalExt,
        _min_y: i8,
        _height: u16,
        _feature: pumpkin_data::placed_feature::PlacedFeature,
        _random: &mut RandomGenerator,
        pos: BlockPos,
    ) -> bool {
        if !chunk.is_air(&pos.0) {
            return false;
        }
        for dir in BlockDirection::all() {
            if dir == BlockDirection::Down
                || !is_acceptable_neighbour(
                    GenerationCache::get_block_state(chunk, &pos.offset(dir.to_offset()).0)
                        .to_state(),
                    dir,
                )
            {
                continue;
            }
            let mut vine =
                pumpkin_data::block_properties::VineLikeProperties::default(&Block::VINE);
            vine.north = dir == BlockDirection::North;
            vine.east = dir == BlockDirection::East;
            vine.south = dir == BlockDirection::South;
            vine.west = dir == BlockDirection::West;
            vine.up = dir == BlockDirection::Up;
            chunk.set_block_state(&pos.0, BlockState::from_id(vine.to_state_id(&Block::VINE)));
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_data::Block;

    use super::is_acceptable_neighbour;

    #[test]
    fn vines_use_the_neighbour_face_instead_of_full_cube_flag() {
        assert!(is_acceptable_neighbour(
            Block::STONE.default_state,
            pumpkin_data::BlockDirection::North
        ));
        assert!(!is_acceptable_neighbour(
            Block::OAK_LEAVES.default_state,
            pumpkin_data::BlockDirection::North
        ));
    }
}
