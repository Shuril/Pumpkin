use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, RandomImpl},
};

use crate::{
    generation::{block_state_provider::BlockStateProvider, proto_chunk::GenerationCache},
    world::WorldPortalExt,
};

pub struct AlterGroundTreeDecorator {
    pub provider: BlockStateProvider,
}

impl AlterGroundTreeDecorator {
    pub fn generate<T: GenerationCache>(
        &self,
        chunk: &mut T,
        block_registry: &dyn WorldPortalExt,
        random: &mut RandomGenerator,
        root_positions: &[BlockPos],
        log_positions: &[BlockPos],
    ) {
        let mut lowest = Vec::with_capacity(root_positions.len() + log_positions.len());
        lowest.extend_from_slice(root_positions);
        lowest.extend_from_slice(log_positions);
        let Some(min_y) = lowest.iter().map(|pos| pos.0.y).min() else {
            return;
        };

        for pos in lowest.into_iter().filter(|pos| pos.0.y == min_y) {
            self.place_circle(chunk, block_registry, random, pos.west().north());
            self.place_circle(chunk, block_registry, random, pos.east().north());
            self.place_circle(chunk, block_registry, random, pos.west().south());
            self.place_circle(chunk, block_registry, random, pos.east().south());
            for _ in 0..5 {
                let placement = random.next_bounded_i32(64);
                let xx = placement % 8;
                let zz = placement / 8;
                if xx == 0 || xx == 7 || zz == 0 || zz == 7 {
                    self.place_circle(
                        chunk,
                        block_registry,
                        random,
                        pos.offset((xx - 3, 0, zz - 3).into()),
                    );
                }
            }
        }
    }

    fn place_circle<T: GenerationCache>(
        &self,
        chunk: &mut T,
        block_registry: &dyn WorldPortalExt,
        random: &mut RandomGenerator,
        center: BlockPos,
    ) {
        for xx in -2i32..=2i32 {
            for zz in -2i32..=2i32 {
                if xx.abs() != 2 || zz.abs() != 2 {
                    self.place_block_at(
                        chunk,
                        block_registry,
                        random,
                        center.offset((xx, 0, zz).into()),
                    );
                }
            }
        }
    }

    fn place_block_at<T: GenerationCache>(
        &self,
        chunk: &mut T,
        block_registry: &dyn WorldPortalExt,
        random: &mut RandomGenerator,
        pos: BlockPos,
    ) {
        for dy in (-3..=2).rev() {
            let cursor = pos.offset((0, dy, 0).into());
            if let Some(state) = self
                .provider
                .get_optional(block_registry, chunk, random, cursor)
            {
                chunk.set_block_state(&cursor.0, state);
                return;
            }
            if !chunk.is_air(&cursor.0) && dy < 0 {
                return;
            }
        }
    }
}
