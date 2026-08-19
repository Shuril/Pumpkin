use std::sync::Arc;

use pumpkin_data::Enchantment;
use pumpkin_data::entity::EntityType;
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::GameMode;

use crate::block::BrokenArgs;
use crate::block::{BlockBehaviour, BlockFuture};
use crate::entity::Entity;

#[pumpkin_block_from_tag("c:cobblestones/infested")]
pub struct InfestedBlock;

impl BlockBehaviour for InfestedBlock {
    fn broken<'a>(&'a self, args: BrokenArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async {
            // Vanilla InfestedBlock.spawnAfterBreak only runs when normal
            // block drops are enabled and the tool does not carry the
            // `prevents_infested_spawns` tag (currently Silk Touch). Creative
            // breaks do not call this side effect in the vanilla path.
            if args.player.gamemode.load() == GameMode::Creative
                || !args.world.level_info.load().game_rules.block_drops
                || args.tool.get_enchantment_level(&Enchantment::SILK_TOUCH) > 0
            {
                return;
            }
            let entity = Entity::new(
                args.world.clone(),
                args.position.0.to_f64(),
                &EntityType::SILVERFISH,
            );

            args.world.spawn_entity(Arc::new(entity)).await;
        })
    }
}
