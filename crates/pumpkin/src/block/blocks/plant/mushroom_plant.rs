use pumpkin_data::tag::Taggable;
use pumpkin_data::{BlockId, BlockStateId, tag};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockAccessor;
use pumpkin_world::world::BlockFlags;
use rand::RngExt;

use crate::block::{
    BlockBehaviour, BlockFuture, BlockMetadata, CanPlaceAtArgs, GetStateForNeighborUpdateArgs,
    blocks::plant::PlantBlockBase,
};

pub struct MushroomPlantBlock;

impl BlockMetadata for MushroomPlantBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::BROWN_MUSHROOM, BlockId::RED_MUSHROOM].into()
    }
}

impl BlockBehaviour for MushroomPlantBlock {
    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        let below = args.position.down();
        let below_state = args.block_accessor.get_block_state(&below);
        let below_block = args.block_accessor.get_block(&below);
        let raw_brightness = args
            .world
            .map_or(0, |world| world.get_raw_brightness(args.position, 0));
        mushroom_can_survive(
            below_block.has_tag(&tag::Block::MINECRAFT_OVERRIDES_MUSHROOM_LIGHT_REQUIREMENT),
            below_state.is_solid_render(),
            raw_brightness,
        )
    }

    fn get_state_for_neighbor_update<'a>(
        &'a self,
        args: GetStateForNeighborUpdateArgs<'a>,
    ) -> BlockFuture<'a, BlockStateId> {
        Box::pin(async move {
            let below = args.position.down();
            let below_state = args.world.get_block_state(&below);
            let below_block = args.world.get_block(&below);
            let survives = mushroom_can_survive(
                below_block.has_tag(&tag::Block::MINECRAFT_OVERRIDES_MUSHROOM_LIGHT_REQUIREMENT),
                below_state.is_solid_render(),
                args.world.get_raw_brightness(args.position, 0),
            );
            if survives {
                args.state_id
            } else {
                pumpkin_data::Block::AIR.default_state.id
            }
        })
    }

    fn random_tick<'a>(&'a self, args: crate::block::RandomTickArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let Some(candidate) = Self::spread_candidate(args.world, args.block, *args.position)
            else {
                return;
            };
            if args.world.get_block_state(&candidate).is_air() {
                args.world
                    .set_block_state(
                        &candidate,
                        args.block.default_state.id,
                        BlockFlags::NOTIFY_LISTENERS,
                    )
                    .await;
            }
        })
    }
}

impl PlantBlockBase for MushroomPlantBlock {
    fn can_plant_on_top(&self, block_accessor: &dyn BlockAccessor, pos: &BlockPos) -> bool {
        let block = block_accessor.get_block(pos);
        block.has_tag(&tag::Block::MINECRAFT_OVERRIDES_MUSHROOM_LIGHT_REQUIREMENT)
    }
}

impl MushroomPlantBlock {
    /// Synchronous part of `randomTick`.  Keeping the thread-local RNG in a
    /// non-async helper is important because `BlockFuture` must be `Send`.
    fn spread_candidate(
        world: &crate::world::World,
        block: &pumpkin_data::Block,
        origin: BlockPos,
    ) -> Option<BlockPos> {
        // MushroomBlock.randomTick: one attempt in 25, with a hard cap of
        // four neighbouring mushrooms in the 9x3x9 search volume.  The
        // mutable candidate position and the four retries deliberately mirror
        // vanilla's sequential algorithm instead of sampling four independent
        // offsets.
        let mut random = rand::rng();
        if random.random_range(0..25) != 0 {
            return None;
        }

        let mut remaining = 5;
        for x in -4..=4 {
            for y in -1..=1 {
                for z in -4..=4 {
                    let probe = origin.offset(Vector3::new(x, y, z));
                    if world.get_block(&probe) == block {
                        remaining -= 1;
                        if remaining <= 0 {
                            return None;
                        }
                    }
                }
            }
        }

        let mut current = origin;
        let mut candidate = origin.offset(Vector3::new(
            random.random_range(0..3) - 1,
            random.random_range(0..2) - random.random_range(0..2),
            random.random_range(0..3) - 1,
        ));
        for _ in 0..4 {
            if world.get_block_state(&candidate).is_air() && Self::can_survive(world, &candidate) {
                current = candidate;
            }
            candidate = current.offset(Vector3::new(
                random.random_range(0..3) - 1,
                random.random_range(0..2) - random.random_range(0..2),
                random.random_range(0..3) - 1,
            ));
        }

        (world.get_block_state(&candidate).is_air() && Self::can_survive(world, &candidate))
            .then_some(candidate)
    }

    fn can_survive(world: &crate::world::World, position: &BlockPos) -> bool {
        let below = position.down();
        let below_state = world.get_block_state(&below);
        let below_block = world.get_block(&below);
        mushroom_can_survive(
            below_block.has_tag(&tag::Block::MINECRAFT_OVERRIDES_MUSHROOM_LIGHT_REQUIREMENT),
            below_state.is_solid_render(),
            world.get_raw_brightness(position, 0),
        )
    }
}

#[inline]
fn mushroom_can_survive(
    support_overrides_light: bool,
    support_is_solid_render: bool,
    raw_brightness: u8,
) -> bool {
    support_overrides_light || (raw_brightness < 13 && support_is_solid_render)
}

#[cfg(test)]
mod tests {
    use super::mushroom_can_survive;

    #[test]
    fn mushroom_light_rule_matches_vanilla_boundary() {
        assert!(mushroom_can_survive(false, true, 0));
        assert!(mushroom_can_survive(false, true, 12));
        assert!(!mushroom_can_survive(false, true, 13));
        assert!(!mushroom_can_survive(false, false, 0));
        assert!(mushroom_can_survive(true, false, 15));
    }
}
