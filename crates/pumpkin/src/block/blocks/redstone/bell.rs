use crate::Arc;
use crate::block::blocks::abstract_wall_mounting::WallMountedBlock;
use crate::block::blocks::redstone::block_receives_redstone_power;
use crate::block::entities::bell::BellBlockEntity;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, BlockFuture, BlockHitResult, BrokenArgs, CanPlaceAtArgs, NormalUseArgs,
    OnNeighborUpdateArgs, OnPlaceArgs, OnProjectileHitArgs, PlacedArgs,
};
use crate::world::World;
use pumpkin_data::BlockStateId;
use pumpkin_data::block_properties::BlockProperties;
use pumpkin_data::block_properties::HorizontalFacing;
use pumpkin_data::block_properties::{AttachFace, BellAttachment, BellLikeProperties};
use pumpkin_data::sound::Sound;
use pumpkin_data::sound::SoundCategory;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockDirection};
use pumpkin_data::{HorizontalFacingExt, tag};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

fn ring_bell(position: BlockPos, world: &Arc<World>, hit_direction: Option<HorizontalFacing>) {
    let (block, state_id) = world.get_block_and_state_id(&position);

    let props = BellLikeProperties::from_state_id(state_id, block);
    let direction = hit_direction.unwrap_or(props.facing);

    if let Some(block_entity) = world.get_block_entity(&position)
        && let Some(be) = block_entity.as_any().downcast_ref::<BellBlockEntity>()
    {
        be.activate(direction);
    }

    world.play_sound_fine(
        Sound::BlockBellUse,
        SoundCategory::Blocks,
        &position.to_centered_f64(),
        1.0,
        2.0,
    );

    let world_for_event = world.clone();
    tokio::spawn(async move {
        world_for_event
            .emit_game_event(
                position,
                crate::world::game_event::GameEventKind::BlockChange,
            )
            .await;
    });
}

#[must_use]
pub fn is_direction_on_bell(
    face: &BlockDirection,
    attachment: BellAttachment,
    block_face: HorizontalFacing,
) -> bool {
    if face == &BlockDirection::Up || face == &BlockDirection::Down {
        return false;
    }
    match attachment {
        BellAttachment::Floor => face.to_axis() == block_face.to_block_direction().to_axis(),
        BellAttachment::SingleWall | BellAttachment::DoubleWall => {
            face.to_axis() != block_face.to_block_direction().to_axis()
        }
        BellAttachment::Ceiling => true,
    }
}

fn is_point_on_bell(
    hit: &BlockHitResult,
    attachment: BellAttachment,
    block_face: HorizontalFacing,
) -> bool {
    if hit.cursor_pos.y <= 0.8124f32 {
        is_direction_on_bell(hit.face, attachment, block_face)
    } else {
        false
    }
}

fn is_single_wall(position: BlockPos, facing: HorizontalFacing, world: &World) -> bool {
    !world
        .get_block(&position.offset(facing.to_offset()))
        .is_solid()
}

#[pumpkin_block("minecraft:bell")]
pub struct BellBlock;

impl WallMountedBlock for BellBlock {
    fn get_direction(&self, state_id: BlockStateId, block: &Block) -> BlockDirection {
        let props = BellLikeProperties::from_state_id(state_id, block);
        match props.attachment {
            BellAttachment::Ceiling => BlockDirection::Down,
            BellAttachment::Floor => BlockDirection::Up,
            BellAttachment::SingleWall | BellAttachment::DoubleWall => {
                props.facing.opposite().to_block_direction()
            }
        }
    }
}

impl BlockBehaviour for BellBlock {
    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        if let Some(direction) = args.direction
            && let Some(world) = args.world
        {
            if direction == BlockDirection::Up {
                let block: &Block = world.get_block(args.position);

                if block.has_tag(&tag::Block::MINECRAFT_UNSTABLE_BOTTOM_CENTER) {
                    false
                } else {
                    let block_pos = args.position.offset(direction.to_offset());
                    let block_state = world.get_block_state(&block_pos);
                    block_state.is_center_solid(direction)
                }
            } else {
                WallMountedBlock::can_place_at(self, world, args.position, direction)
            }
        } else {
            false
        }
    }
    fn broken<'a>(&'a self, args: BrokenArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let world: &World = args.world;
            world.remove_block_entity(args.position);
        })
    }

    fn placed<'a>(&'a self, args: PlacedArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            args.world
                .add_block_entity(Arc::new(BellBlockEntity::new(*args.position)));
        })
    }
    fn normal_use<'a>(&'a self, args: NormalUseArgs<'a>) -> BlockFuture<'a, BlockActionResult> {
        Box::pin(async move {
            let state = args.world.get_block_state(args.position);

            let props = BellLikeProperties::from_state_id(state.id, args.block);

            if !is_point_on_bell(args.hit, props.attachment, props.facing) {
                return BlockActionResult::Pass; // Pass if Crosshair wasn't correctly positioned
            }
            ring_bell(
                *args.position,
                args.world,
                args.hit.face.to_horizontal_facing(),
            );

            args.player
                .increment_stat(
                    pumpkin_data::statistic::StatisticCategory::Custom,
                    pumpkin_data::statistic::CustomStatistic::BellRing as i32,
                    1,
                )
                .await;

            BlockActionResult::Success
        })
    }

    fn on_place<'a>(&'a self, args: OnPlaceArgs<'a>) -> BlockFuture<'a, BlockStateId> {
        Box::pin(async move {
            let mut props = BellLikeProperties::default(args.block);

            let block_face;
            let facing;
            (block_face, facing) =
                WallMountedBlock::get_placement_face(self, args.player, args.direction);

            props.facing = match block_face {
                AttachFace::Floor | AttachFace::Ceiling => facing,
                AttachFace::Wall => facing.opposite(),
            };

            props.attachment = match block_face {
                AttachFace::Wall => {
                    if is_single_wall(*args.position, props.facing.opposite(), args.world) {
                        BellAttachment::SingleWall
                    } else {
                        BellAttachment::DoubleWall
                    }
                }
                AttachFace::Floor => BellAttachment::Floor,
                AttachFace::Ceiling => BellAttachment::Ceiling,
            };

            props.to_state_id(args.block)
        })
    }

    fn on_neighbor_update<'a>(&'a self, args: OnNeighborUpdateArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let world: &World = args.world;

            let is_receiving_power = block_receives_redstone_power(world, args.position).await;
            let state = args.world.get_block_state(args.position);

            let mut props = BellLikeProperties::from_state_id(state.id, args.block);

            if props.powered != is_receiving_power {
                props.powered = is_receiving_power;

                args.world
                    .set_block_state(
                        args.position,
                        props.to_state_id(args.block),
                        BlockFlags::NOTIFY_ALL,
                    )
                    .await;

                if is_receiving_power {
                    ring_bell(*args.position, args.world, None);
                }
            }
        })
    }

    fn on_projectile_hit<'a>(&'a self, args: OnProjectileHitArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let crate::entity::projectile::ProjectileHit::Block { face, .. } = args.hit else {
                return;
            };

            let props = BellLikeProperties::from_state_id(args.state.id, args.block);
            if !is_direction_on_bell(face, props.attachment, props.facing) {
                return;
            }

            ring_bell(*args.position, args.world, face.to_horizontal_facing());

            if let Some(owner_id) = args.projectile.projectile_owner_id()
                && let Some(player) = args.world.get_player_by_id(owner_id)
            {
                player
                    .increment_stat(
                        pumpkin_data::statistic::StatisticCategory::Custom,
                        pumpkin_data::statistic::CustomStatistic::BellRing as i32,
                        1,
                    )
                    .await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bell_direction_detection() {
        // Vertical hit directions are never valid
        assert!(!is_direction_on_bell(
            &BlockDirection::Up,
            BellAttachment::Floor,
            HorizontalFacing::North
        ));
        assert!(!is_direction_on_bell(
            &BlockDirection::Down,
            BellAttachment::Ceiling,
            HorizontalFacing::North
        ));

        // Floor: hit face axis must match block facing axis
        assert!(is_direction_on_bell(
            &BlockDirection::North,
            BellAttachment::Floor,
            HorizontalFacing::North
        ));
        assert!(is_direction_on_bell(
            &BlockDirection::South,
            BellAttachment::Floor,
            HorizontalFacing::North
        ));
        assert!(!is_direction_on_bell(
            &BlockDirection::East,
            BellAttachment::Floor,
            HorizontalFacing::North
        ));

        // Wall: hit face axis must NOT match block facing axis
        assert!(is_direction_on_bell(
            &BlockDirection::East,
            BellAttachment::SingleWall,
            HorizontalFacing::North
        ));
        assert!(is_direction_on_bell(
            &BlockDirection::West,
            BellAttachment::DoubleWall,
            HorizontalFacing::North
        ));
        assert!(!is_direction_on_bell(
            &BlockDirection::North,
            BellAttachment::SingleWall,
            HorizontalFacing::North
        ));

        // Ceiling: any horizontal direction is valid
        assert!(is_direction_on_bell(
            &BlockDirection::North,
            BellAttachment::Ceiling,
            HorizontalFacing::North
        ));
        assert!(is_direction_on_bell(
            &BlockDirection::East,
            BellAttachment::Ceiling,
            HorizontalFacing::North
        ));
    }
}
