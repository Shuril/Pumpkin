use pumpkin_data::{BlockDirection, fluid::Fluid};
use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, RandomImpl},
};

use crate::generation::block_predicate::BlockPredicate;
use crate::generation::block_state_provider::BlockStateProvider;
use crate::generation::proto_chunk::GenerationCache;
use crate::world::WorldPortalExt;

pub struct RootSystemFeature {
    pub feature: Box<crate::generation::feature::placed_features::PlacedFeature>,
    pub required_vertical_space_for_tree: i32,
    pub root_radius: i32,
    pub root_replaceable: BlockPredicate,
    pub root_state_provider: BlockStateProvider,
    pub root_placement_attempts: i32,
    pub root_column_max_height: i32,
    pub hanging_root_radius: i32,
    pub hanging_roots_vertical_span: i32,
    pub hanging_root_state_provider: BlockStateProvider,
    pub hanging_root_placement_attempts: i32,
    pub allowed_vertical_water_for_tree: i32,
    pub allowed_tree_position: BlockPredicate,
}

impl RootSystemFeature {
    #[allow(clippy::too_many_arguments)]
    pub fn generate<T: GenerationCache>(
        &self,
        chunk: &mut T,
        block_registry: &dyn WorldPortalExt,
        min_y: i8,
        height: u16,
        feature_name: pumpkin_data::placed_feature::PlacedFeature,
        random: &mut RandomGenerator,
        pos: BlockPos,
    ) -> bool {
        if !self.allowed_tree_position.test(block_registry, chunk, &pos) {
            return false;
        }

        if !self.can_place_tree(chunk, pos) {
            return false;
        }

        if !self.feature.generate(
            chunk,
            block_registry,
            min_y,
            height,
            feature_name,
            random,
            pos,
        ) {
            return false;
        }

        self.place_roots(chunk, block_registry, random, pos);
        true
    }

    fn can_place_tree<T: GenerationCache>(&self, chunk: &T, pos: BlockPos) -> bool {
        let mut mutable_pos = pos;
        for i in 1..=self.required_vertical_space_for_tree {
            mutable_pos = mutable_pos.add(0, 1, 0);
            if !self.is_allowed_tree_space(chunk, mutable_pos, i) {
                return false;
            }
        }
        true
    }

    fn is_allowed_tree_space<T: GenerationCache>(
        &self,
        chunk: &T,
        pos: BlockPos,
        vertical_space: i32,
    ) -> bool {
        let state_is_air = chunk.is_air(&pos.0);
        let (fluid, _) = GenerationCache::get_fluid_and_fluid_state(chunk, &pos.0);
        tree_space_allowed(
            state_is_air,
            fluid == Fluid::WATER,
            vertical_space,
            self.allowed_vertical_water_for_tree,
        )
    }

    fn place_roots<T: GenerationCache>(
        &self,
        chunk: &mut T,
        block_registry: &dyn WorldPortalExt,
        random: &mut RandomGenerator,
        pos: BlockPos,
    ) {
        self.place_rooted_dirt(chunk, block_registry, random, pos);
        self.place_hanging_roots(chunk, block_registry, random, pos);
    }

    fn place_rooted_dirt<T: GenerationCache>(
        &self,
        chunk: &mut T,
        block_registry: &dyn WorldPortalExt,
        random: &mut RandomGenerator,
        pos: BlockPos,
    ) {
        for _ in 0..self.root_placement_attempts {
            let mut mutable_pos = pos.add(
                random.next_bounded_i32(self.root_radius.max(1))
                    - random.next_bounded_i32(self.root_radius.max(1)),
                0,
                random.next_bounded_i32(self.root_radius.max(1))
                    - random.next_bounded_i32(self.root_radius.max(1)),
            );

            if self
                .root_replaceable
                .test(block_registry, chunk, &mutable_pos)
            {
                chunk.set_block_state(
                    &mutable_pos.0,
                    self.root_state_provider.get_with_context(
                        block_registry,
                        chunk,
                        random,
                        mutable_pos,
                    ),
                );
            }

            for _ in 0..self.root_column_max_height {
                mutable_pos = mutable_pos.add(0, -1, 0);
                if chunk.out_of_height(mutable_pos.0.y as i16)
                    || !self
                        .root_replaceable
                        .test(block_registry, chunk, &mutable_pos)
                {
                    break;
                }
                chunk.set_block_state(
                    &mutable_pos.0,
                    self.root_state_provider.get_with_context(
                        block_registry,
                        chunk,
                        random,
                        mutable_pos,
                    ),
                );
            }
        }
    }

    fn place_hanging_roots<T: GenerationCache>(
        &self,
        chunk: &mut T,
        block_registry: &dyn WorldPortalExt,
        random: &mut RandomGenerator,
        pos: BlockPos,
    ) {
        for _ in 0..self.hanging_root_placement_attempts {
            let mutable_pos = pos.add(
                random.next_bounded_i32(self.hanging_root_radius.max(1))
                    - random.next_bounded_i32(self.hanging_root_radius.max(1)),
                random.next_bounded_i32(self.hanging_roots_vertical_span.max(1))
                    - random.next_bounded_i32(self.hanging_roots_vertical_span.max(1)),
                random.next_bounded_i32(self.hanging_root_radius.max(1))
                    - random.next_bounded_i32(self.hanging_root_radius.max(1)),
            );

            if chunk.is_air(&mutable_pos.0) {
                let state = self.hanging_root_state_provider.get_with_context(
                    block_registry,
                    chunk,
                    random,
                    mutable_pos,
                );
                let above = mutable_pos.add(0, 1, 0);
                let above_state = GenerationCache::get_block_state(chunk, &above.0).to_state();
                // Rooted/hanging roots use the same survival contract as the
                // vanilla block: the supporting block must expose a sturdy
                // downward face, not merely be non-air (water, plants and
                // panes are not valid supports).
                if above_state.is_side_solid(BlockDirection::Down) {
                    chunk.set_block_state(&mutable_pos.0, state);
                }
            }
        }
    }
}

/// Vanilla `RootSystemFeature.isAllowedTreeSpace`.
///
/// `vertical_space` is one-based (`1` is the first block above the origin),
/// while the water allowance is expressed as the number of blocks above the
/// ground.  Therefore the comparison intentionally uses `+ 1`.
#[inline]
const fn tree_space_allowed(
    state_is_air: bool,
    fluid_is_water: bool,
    vertical_space: i32,
    allowed_vertical_water: i32,
) -> bool {
    state_is_air || (vertical_space + 1 <= allowed_vertical_water && fluid_is_water)
}

#[cfg(test)]
mod tests {
    use super::tree_space_allowed;

    #[test]
    fn root_tree_water_allowance_uses_vanilla_one_based_height() {
        assert!(tree_space_allowed(false, true, 1, 2));
        assert!(!tree_space_allowed(false, true, 2, 2));
        assert!(!tree_space_allowed(false, false, 1, 2));
        assert!(tree_space_allowed(true, false, 99, 0));
    }
}
