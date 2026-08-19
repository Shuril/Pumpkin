use crate::generation::proto_chunk::GenerationCache;
use pumpkin_data::BlockDirection;
use pumpkin_data::BlockId;
use pumpkin_data::tag;
use pumpkin_data::{
    Block, BlockState, BlockStateId,
    block_properties::{
        BlockProperties, PointedDripstoneLikeProperties, SpeleothemThickness, VerticalDirection,
    },
};
use pumpkin_util::{
    math::{position::BlockPos, vector3::Vector3},
    random::{RandomGenerator, RandomImpl},
};

pub mod cluster;
pub mod large;
pub mod small;

pub(super) fn can_replace(id: BlockId) -> bool {
    id == BlockId::DRIPSTONE_BLOCK || id.has_tag(tag::Block::MINECRAFT_DRIPSTONE_REPLACEABLE_BLOCKS)
}

pub(super) fn gen_dripstone<T: GenerationCache>(chunk: &mut T, pos: BlockPos) -> bool {
    let block = GenerationCache::get_block_state(chunk, &pos.0).to_block_id();
    if block.has_tag(tag::Block::MINECRAFT_DRIPSTONE_REPLACEABLE_BLOCKS) {
        chunk.set_block_state(&pos.0, Block::DRIPSTONE_BLOCK.default_state);
        return true;
    }
    false
}

/// Returns whether a generated pointed-dripstone position can hold a pointed
/// block.  Vanilla treats both air and water as empty here; water is retained
/// as the pointed block's `waterlogged` property rather than being replaced by
/// a dry state.
#[inline]
fn is_empty_or_water<T: GenerationCache>(chunk: &T, pos: &Vector3<i32>) -> bool {
    let state = BlockState::from_id(GenerationCache::get_block_state(chunk, pos));
    state.is_air() || state.id.to_block() == &Block::WATER
}

#[inline]
fn pointed_state(
    direction: BlockDirection,
    thickness: SpeleothemThickness,
    waterlogged: bool,
) -> BlockStateId {
    let mut properties = PointedDripstoneLikeProperties::default(&Block::POINTED_DRIPSTONE);
    properties.vertical_direction = match direction {
        BlockDirection::Up => VerticalDirection::Up,
        BlockDirection::Down => VerticalDirection::Down,
        _ => unreachable!("pointed dripstone can only grow vertically"),
    };
    properties.thickness = thickness;
    properties.waterlogged = waterlogged;
    properties.to_state_id(&Block::POINTED_DRIPSTONE)
}

/// Builds the pointed-dripstone column from its base toward the tip.  This is
/// the shared `SpeleothemUtils.buildBaseToTipColumn` algorithm from vanilla:
/// columns of length one are a tip, length two are frustum+tip, and longer
/// columns are base+middle*+frustum+tip.
fn grow_pointed_dripstone<T: GenerationCache>(
    chunk: &mut T,
    start: BlockPos,
    direction: BlockDirection,
    height: i32,
) {
    let base = start.offset(direction.opposite().to_offset());
    if !can_replace(GenerationCache::get_block_state(chunk, &base.0).to_block_id()) {
        return;
    }

    let mut current = start;
    let mut place = |thickness: SpeleothemThickness| {
        let state = GenerationCache::get_block_state(chunk, &current.0);
        let waterlogged = BlockState::from_id(state).id.to_block() == &Block::WATER;
        chunk.set_block_state(
            &current.0,
            BlockState::from_id(pointed_state(direction, thickness, waterlogged)),
        );
        current = current.offset(direction.to_offset());
    };

    if height >= 3 {
        place(SpeleothemThickness::Base);
        for _ in 0..height - 3 {
            place(SpeleothemThickness::Middle);
        }
    }
    if height >= 2 {
        place(SpeleothemThickness::Frustum);
    }
    if height >= 1 {
        place(SpeleothemThickness::Tip);
    }
}

/// Places the pointed-dripstone column at a feature origin.  The origin is
/// deliberately checked before choosing the taller variant, matching
/// `PointedDripstoneFeature.place`: a second block is only allowed when the
/// position immediately beyond the tip is air or water.
pub(super) fn grow_pointed_feature<T: GenerationCache>(
    chunk: &mut T,
    origin: BlockPos,
    direction: BlockDirection,
    chance_of_taller: f32,
    random: &mut RandomGenerator,
) {
    let next = origin.offset(direction.to_offset());
    let height = if random.next_f32() < chance_of_taller && is_empty_or_water(chunk, &next.0) {
        2
    } else {
        1
    };
    grow_pointed_dripstone(chunk, origin, direction, height);
}

#[cfg(test)]
mod tests {
    use super::{SpeleothemThickness, pointed_state};
    use pumpkin_data::{
        Block, BlockDirection,
        block_properties::{BlockProperties, PointedDripstoneLikeProperties, VerticalDirection},
    };

    #[test]
    fn pointed_column_states_follow_vanilla_order() {
        let base = PointedDripstoneLikeProperties::from_state_id(
            pointed_state(BlockDirection::Down, SpeleothemThickness::Base, false),
            &Block::POINTED_DRIPSTONE,
        );
        let middle = PointedDripstoneLikeProperties::from_state_id(
            pointed_state(BlockDirection::Down, SpeleothemThickness::Middle, false),
            &Block::POINTED_DRIPSTONE,
        );
        let frustum = PointedDripstoneLikeProperties::from_state_id(
            pointed_state(BlockDirection::Down, SpeleothemThickness::Frustum, false),
            &Block::POINTED_DRIPSTONE,
        );
        let tip = PointedDripstoneLikeProperties::from_state_id(
            pointed_state(BlockDirection::Down, SpeleothemThickness::Tip, true),
            &Block::POINTED_DRIPSTONE,
        );
        assert_eq!(base.vertical_direction, VerticalDirection::Down);
        assert_eq!(base.thickness, SpeleothemThickness::Base);
        assert_eq!(middle.thickness, SpeleothemThickness::Middle);
        assert_eq!(frustum.thickness, SpeleothemThickness::Frustum);
        assert_eq!(tip.thickness, SpeleothemThickness::Tip);
        assert!(tip.waterlogged);
    }
}
