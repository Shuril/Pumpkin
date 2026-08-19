use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::block::UseWithItemArgs;
use crate::block::entities::BlockEntity;
use crate::block::entities::sign::SignBlockEntity;
use crate::block::registry::BlockActionResult;
use crate::entity::player::Player;
use crate::item::items::state_with_properties_of;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use crate::world::World;
use pumpkin_data::Block;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{BlockDirection, BlockId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;

pub struct HoneyCombItem;

impl ItemMetadata for HoneyCombItem {
    fn ids() -> Box<[u16]> {
        [Item::HONEYCOMB.id].into()
    }
}

impl ItemBehaviour for HoneyCombItem {
    fn use_on_block<'a>(
        &'a self,
        item: &'a mut ItemStack,
        player: &'a Player,
        location: BlockPos,
        _face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        block: &'a Block,
        _server: &'a Server,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let world = player.world();
            if try_wax_block(&world, location, block).await {
                item.decrement_unless_creative(player.gamemode.load(), 1);
            }
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Waxes the block at `location` if it has a waxed equivalent, emitting the wax
/// particles and sound on success.
pub(crate) async fn try_wax_block(world: &Arc<World>, location: BlockPos, block: &Block) -> bool {
    let Some(replacement) = get_waxed_equivalent(block.id) else {
        return false;
    };
    let new_block = replacement.to_block();

    let old_state_id = world.get_block_state_id(&location);
    let new_state_id = state_with_properties_of(block, old_state_id, new_block);

    world
        .set_block_state(&location, new_state_id, BlockFlags::NOTIFY_ALL)
        .await;
    world.sync_world_event(WorldEvent::ParticlesAndSoundWaxOn, location, 0);
    true
}

impl HoneyCombItem {
    pub fn apply_to_sign(
        &self,
        args: &UseWithItemArgs<'_>,
        block_entity: &Arc<dyn BlockEntity>,
        sign_entity: &SignBlockEntity,
    ) -> BlockActionResult {
        sign_entity.is_waxed.store(true, Ordering::Relaxed);

        args.world.update_block_entity(block_entity);
        args.world
            .sync_world_event(WorldEvent::ParticlesAndSoundWaxOn, *args.position, 0);

        BlockActionResult::Success
    }
}

const fn get_waxed_equivalent(id: BlockId) -> Option<BlockId> {
    match id {
        BlockId::OXIDIZED_COPPER => Some(BlockId::WAXED_OXIDIZED_COPPER),
        BlockId::WEATHERED_COPPER => Some(BlockId::WAXED_WEATHERED_COPPER),
        BlockId::EXPOSED_COPPER => Some(BlockId::WAXED_EXPOSED_COPPER),
        BlockId::COPPER_BLOCK => Some(BlockId::WAXED_COPPER_BLOCK),
        BlockId::OXIDIZED_CHISELED_COPPER => Some(BlockId::WAXED_OXIDIZED_CHISELED_COPPER),
        BlockId::WEATHERED_CHISELED_COPPER => Some(BlockId::WAXED_WEATHERED_CHISELED_COPPER),
        BlockId::EXPOSED_CHISELED_COPPER => Some(BlockId::WAXED_EXPOSED_CHISELED_COPPER),
        BlockId::CHISELED_COPPER => Some(BlockId::WAXED_CHISELED_COPPER),
        BlockId::OXIDIZED_COPPER_GRATE => Some(BlockId::WAXED_OXIDIZED_COPPER_GRATE),
        BlockId::WEATHERED_COPPER_GRATE => Some(BlockId::WAXED_WEATHERED_COPPER_GRATE),
        BlockId::EXPOSED_COPPER_GRATE => Some(BlockId::WAXED_EXPOSED_COPPER_GRATE),
        BlockId::COPPER_GRATE => Some(BlockId::WAXED_COPPER_GRATE),
        BlockId::OXIDIZED_CUT_COPPER => Some(BlockId::WAXED_OXIDIZED_CUT_COPPER),
        BlockId::WEATHERED_CUT_COPPER => Some(BlockId::WAXED_WEATHERED_CUT_COPPER),
        BlockId::EXPOSED_CUT_COPPER => Some(BlockId::WAXED_EXPOSED_CUT_COPPER),
        BlockId::CUT_COPPER => Some(BlockId::WAXED_CUT_COPPER),
        BlockId::OXIDIZED_CUT_COPPER_STAIRS => Some(BlockId::WAXED_OXIDIZED_CUT_COPPER_STAIRS),
        BlockId::WEATHERED_CUT_COPPER_STAIRS => Some(BlockId::WAXED_WEATHERED_CUT_COPPER_STAIRS),
        BlockId::EXPOSED_CUT_COPPER_STAIRS => Some(BlockId::WAXED_EXPOSED_CUT_COPPER_STAIRS),
        BlockId::CUT_COPPER_STAIRS => Some(BlockId::WAXED_CUT_COPPER_STAIRS),
        BlockId::OXIDIZED_CUT_COPPER_SLAB => Some(BlockId::WAXED_OXIDIZED_CUT_COPPER_SLAB),
        BlockId::WEATHERED_CUT_COPPER_SLAB => Some(BlockId::WAXED_WEATHERED_CUT_COPPER_SLAB),
        BlockId::EXPOSED_CUT_COPPER_SLAB => Some(BlockId::WAXED_EXPOSED_CUT_COPPER_SLAB),
        BlockId::CUT_COPPER_SLAB => Some(BlockId::WAXED_CUT_COPPER_SLAB),
        BlockId::OXIDIZED_COPPER_BULB => Some(BlockId::WAXED_OXIDIZED_COPPER_BULB),
        BlockId::WEATHERED_COPPER_BULB => Some(BlockId::WAXED_WEATHERED_COPPER_BULB),
        BlockId::EXPOSED_COPPER_BULB => Some(BlockId::WAXED_EXPOSED_COPPER_BULB),
        BlockId::COPPER_BULB => Some(BlockId::WAXED_COPPER_BULB),
        BlockId::OXIDIZED_COPPER_DOOR => Some(BlockId::WAXED_OXIDIZED_COPPER_DOOR),
        BlockId::WEATHERED_COPPER_DOOR => Some(BlockId::WAXED_WEATHERED_COPPER_DOOR),
        BlockId::EXPOSED_COPPER_DOOR => Some(BlockId::WAXED_EXPOSED_COPPER_DOOR),
        BlockId::COPPER_DOOR => Some(BlockId::WAXED_COPPER_DOOR),
        BlockId::OXIDIZED_COPPER_TRAPDOOR => Some(BlockId::WAXED_OXIDIZED_COPPER_TRAPDOOR),
        BlockId::WEATHERED_COPPER_TRAPDOOR => Some(BlockId::WAXED_WEATHERED_COPPER_TRAPDOOR),
        BlockId::EXPOSED_COPPER_TRAPDOOR => Some(BlockId::WAXED_EXPOSED_COPPER_TRAPDOOR),
        BlockId::COPPER_TRAPDOOR => Some(BlockId::WAXED_COPPER_TRAPDOOR),
        BlockId::OXIDIZED_COPPER_BARS => Some(BlockId::WAXED_OXIDIZED_COPPER_BARS),
        BlockId::WEATHERED_COPPER_BARS => Some(BlockId::WAXED_WEATHERED_COPPER_BARS),
        BlockId::EXPOSED_COPPER_BARS => Some(BlockId::WAXED_EXPOSED_COPPER_BARS),
        BlockId::COPPER_BARS => Some(BlockId::WAXED_COPPER_BARS),
        BlockId::OXIDIZED_COPPER_CHAIN => Some(BlockId::WAXED_OXIDIZED_COPPER_CHAIN),
        BlockId::WEATHERED_COPPER_CHAIN => Some(BlockId::WAXED_WEATHERED_COPPER_CHAIN),
        BlockId::EXPOSED_COPPER_CHAIN => Some(BlockId::WAXED_EXPOSED_COPPER_CHAIN),
        BlockId::COPPER_CHAIN => Some(BlockId::WAXED_COPPER_CHAIN),
        BlockId::OXIDIZED_COPPER_CHEST => Some(BlockId::WAXED_OXIDIZED_COPPER_CHEST),
        BlockId::WEATHERED_COPPER_CHEST => Some(BlockId::WAXED_WEATHERED_COPPER_CHEST),
        BlockId::EXPOSED_COPPER_CHEST => Some(BlockId::WAXED_EXPOSED_COPPER_CHEST),
        BlockId::COPPER_CHEST => Some(BlockId::WAXED_COPPER_CHEST),
        BlockId::OXIDIZED_COPPER_GOLEM_STATUE => Some(BlockId::WAXED_OXIDIZED_COPPER_GOLEM_STATUE),
        BlockId::WEATHERED_COPPER_GOLEM_STATUE => {
            Some(BlockId::WAXED_WEATHERED_COPPER_GOLEM_STATUE)
        }
        BlockId::EXPOSED_COPPER_GOLEM_STATUE => Some(BlockId::WAXED_EXPOSED_COPPER_GOLEM_STATUE),
        BlockId::COPPER_GOLEM_STATUE => Some(BlockId::WAXED_COPPER_GOLEM_STATUE),
        BlockId::OXIDIZED_COPPER_LANTERN => Some(BlockId::WAXED_OXIDIZED_COPPER_LANTERN),
        BlockId::WEATHERED_COPPER_LANTERN => Some(BlockId::WAXED_WEATHERED_COPPER_LANTERN),
        BlockId::EXPOSED_COPPER_LANTERN => Some(BlockId::WAXED_EXPOSED_COPPER_LANTERN),
        BlockId::COPPER_LANTERN => Some(BlockId::WAXED_COPPER_LANTERN),
        BlockId::OXIDIZED_LIGHTNING_ROD => Some(BlockId::WAXED_OXIDIZED_LIGHTNING_ROD),
        BlockId::WEATHERED_LIGHTNING_ROD => Some(BlockId::WAXED_WEATHERED_LIGHTNING_ROD),
        BlockId::EXPOSED_LIGHTNING_ROD => Some(BlockId::WAXED_EXPOSED_LIGHTNING_ROD),
        BlockId::LIGHTNING_ROD => Some(BlockId::WAXED_LIGHTNING_ROD),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::get_waxed_equivalent;
    use pumpkin_data::BlockId;

    #[test]
    fn every_copper_collection_has_a_waxed_equivalent() {
        let pairs = [
            (BlockId::COPPER_BARS, BlockId::WAXED_COPPER_BARS),
            (BlockId::COPPER_CHAIN, BlockId::WAXED_COPPER_CHAIN),
            (BlockId::COPPER_CHEST, BlockId::WAXED_COPPER_CHEST),
            (
                BlockId::COPPER_GOLEM_STATUE,
                BlockId::WAXED_COPPER_GOLEM_STATUE,
            ),
            (BlockId::COPPER_LANTERN, BlockId::WAXED_COPPER_LANTERN),
            (BlockId::LIGHTNING_ROD, BlockId::WAXED_LIGHTNING_ROD),
            (
                BlockId::EXPOSED_COPPER_BARS,
                BlockId::WAXED_EXPOSED_COPPER_BARS,
            ),
            (
                BlockId::WEATHERED_COPPER_CHEST,
                BlockId::WAXED_WEATHERED_COPPER_CHEST,
            ),
            (
                BlockId::OXIDIZED_LIGHTNING_ROD,
                BlockId::WAXED_OXIDIZED_LIGHTNING_ROD,
            ),
        ];
        for (unwaxed, waxed) in pairs {
            assert_eq!(get_waxed_equivalent(unwaxed), Some(waxed));
        }
    }
}
