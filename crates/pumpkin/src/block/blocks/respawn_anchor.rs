use pumpkin_data::block_properties::{BlockProperties, RespawnAnchorLikeProperties};
use pumpkin_data::dimension::Dimension;
use pumpkin_data::item::Item;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::translation;
use pumpkin_macros::pumpkin_block;
use pumpkin_world::world::BlockFlags;

use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, BlockFuture, GetComparatorOutputArgs, NormalUseArgs, UseWithItemArgs,
};
use crate::entity::EntityBase;

#[pumpkin_block("minecraft:respawn_anchor")]
pub struct RespawnAnchorBlock;

impl BlockBehaviour for RespawnAnchorBlock {
    fn use_with_item<'a>(
        &'a self,
        args: UseWithItemArgs<'a>,
    ) -> BlockFuture<'a, BlockActionResult> {
        Box::pin(async move {
            if args.item_stack.item.id != Item::GLOWSTONE.id {
                return BlockActionResult::Pass;
            }

            if !Self::charge(args.world, args.position, args.block).await {
                return BlockActionResult::Pass;
            }

            args.item_stack
                .decrement_unless_creative(args.player.gamemode.load(), 1);

            BlockActionResult::Success
        })
    }

    fn normal_use<'a>(&'a self, args: NormalUseArgs<'a>) -> BlockFuture<'a, BlockActionResult> {
        Box::pin(async move {
            let state_id = args.world.get_block_state_id(args.position);
            let props = RespawnAnchorLikeProperties::from_state_id(state_id, args.block);

            if args.world.dimension != Dimension::THE_NETHER {
                args.world
                    .break_block(args.position, None, BlockFlags::SKIP_DROPS)
                    .await;
                args.world
                    .explode(args.position.to_centered_f64(), 5.0)
                    .await;
                return BlockActionResult::SuccessServer;
            }

            if props.charges == 0 {
                args.player
                    .send_system_message(&pumpkin_macros::translate_cross!(
                        translation::java::BLOCK_MINECRAFT_BED_NO_SLEEP,
                        translation::bedrock::TILE_BED_NOSLEEP
                    ))
                    .await;
                return BlockActionResult::SuccessServer;
            }

            if args
                .player
                .set_respawn_point(
                    args.world.dimension.clone(),
                    *args.position,
                    args.player.get_entity().yaw.load(),
                    args.player.get_entity().pitch.load(),
                    false,
                )
                .await
            {
                args.world.play_sound(
                    Sound::BlockRespawnAnchorSetSpawn,
                    SoundCategory::Blocks,
                    &args.position.to_f64(),
                );

                args.player
                    .send_system_message(&pumpkin_macros::translate_cross!(
                        translation::java::BLOCK_MINECRAFT_SET_SPAWN,
                        translation::bedrock::TILE_BED_RESPAWNSET
                    ))
                    .await;
            }

            BlockActionResult::SuccessServer
        })
    }

    fn get_comparator_output<'a>(
        &'a self,
        args: GetComparatorOutputArgs<'a>,
    ) -> BlockFuture<'a, Option<u8>> {
        Box::pin(async move {
            let state_id = args.world.get_block_state_id(args.position);
            let props = RespawnAnchorLikeProperties::from_state_id(state_id, args.block);
            Some(Self::get_scaled_charge_level(props.charges))
        })
    }
}

impl RespawnAnchorBlock {
    #[must_use]
    pub const fn get_scaled_charge_level(charges: u8) -> u8 {
        // Vanilla: floor((charges / 4.0) * 15)
        // 0 -> 0, 1 -> 3, 2 -> 7, 3 -> 11, 4 -> 15
        match charges {
            0 => 0,
            1 => 3,
            2 => 7,
            3 => 11,
            _ => 15,
        }
    }

    /// Applies the block-only part of `GlowstoneBlock`'s dispenser behavior.
    /// The item is consumed by the caller only after this returns true, so a
    /// full anchor, another dimension, or a missing target is a normal failed
    /// dispense and preserves the stack for fallback ejection.
    pub async fn charge(
        world: &std::sync::Arc<crate::world::World>,
        position: &pumpkin_util::math::position::BlockPos,
        block: &pumpkin_data::Block,
    ) -> bool {
        if world.dimension != Dimension::THE_NETHER {
            return false;
        }
        let state_id = world.get_block_state_id(position);
        let mut props = RespawnAnchorLikeProperties::from_state_id(state_id, block);
        if props.charges >= 4 {
            return false;
        }
        props.charges += 1;
        world
            .set_block_state(position, props.to_state_id(block), BlockFlags::NOTIFY_ALL)
            .await;
        world.update_neighbors(position, None).await;
        world.update_comparators(position, block).await;
        world
            .emit_game_event(
                *position,
                crate::world::game_event::GameEventKind::BlockChange,
            )
            .await;
        world.play_sound(
            Sound::BlockRespawnAnchorCharge,
            SoundCategory::Blocks,
            &position.to_f64(),
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_respawn_anchor_comparator_levels() {
        assert_eq!(RespawnAnchorBlock::get_scaled_charge_level(0), 0);
        assert_eq!(RespawnAnchorBlock::get_scaled_charge_level(1), 3);
        assert_eq!(RespawnAnchorBlock::get_scaled_charge_level(2), 7);
        assert_eq!(RespawnAnchorBlock::get_scaled_charge_level(3), 11);
        assert_eq!(RespawnAnchorBlock::get_scaled_charge_level(4), 15);
    }
}
