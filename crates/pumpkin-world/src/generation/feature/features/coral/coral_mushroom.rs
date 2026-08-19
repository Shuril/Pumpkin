use crate::{generation::proto_chunk::GenerationCache, world::WorldPortalExt};
use pumpkin_data::tag;
use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, RandomImpl},
};

use super::CoralFeature;

pub struct CoralMushroomFeature;

impl CoralMushroomFeature {
    #[allow(clippy::too_many_arguments)]
    pub fn generate<T: GenerationCache>(
        chunk: &mut T,
        block_registry: &dyn WorldPortalExt,
        _min_y: i8,
        _height: u16,
        _feature: pumpkin_data::placed_feature::PlacedFeature, // This placed feature
        random: &mut RandomGenerator,
        pos: BlockPos,
    ) -> bool {
        // First lets get a random coral
        let block = CoralFeature::get_random_tag_entry(tag::Block::MINECRAFT_CORAL_BLOCKS, random);

        let height = random.next_bounded_i32(3) + 3;
        let width = random.next_bounded_i32(3) + 3;
        let length = random.next_bounded_i32(3) + 3;
        let sink = random.next_bounded_i32(3) + 1;

        // This is the direct shape predicate from vanilla's
        // CoralMushroomFeature.placeFeature.  In particular, the four corner
        // columns are skipped, while shell/interior cells may be filled.  The
        // previous port both used the wrong predicate and added the origin to
        // an already absolute position, moving every mushroom away from its
        // feature origin.
        for x in 0..=width {
            for y in 0..=height {
                for z in 0..=length {
                    let candidate =
                        BlockPos::new(pos.0.x + x, pos.0.y + y, pos.0.z + z).down_height(sink);
                    let corner_xy = (x == 0 || x == width) && (y == 0 || y == height);
                    let shell_or_interior = (z != 0 && z != length) || (y != 0 && y != height);
                    let edge_xz = (x == 0 || x == width) && (z == 0 || z == length);
                    let interior = (x != 0 && x != width)
                        && (y != 0 && y != height)
                        && (z != 0 && z != length);

                    if corner_xy
                        || (shell_or_interior
                            && (edge_xz
                                || interior
                                || random.next_f32() < 0.1
                                || CoralFeature::generate_coral_piece(
                                    chunk,
                                    block_registry,
                                    random,
                                    block,
                                    candidate,
                                )))
                    {
                        // The Java implementation intentionally has an empty
                        // body here: the final disjunct performs placement and
                        // all other branches are just shape gating.
                    }
                }
            }
        }
        true
    }
}
