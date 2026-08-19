use std::sync::Arc;

use crate::block::entities::sculk_shrieker::SculkShriekerBlockEntity;
use crate::block::{
    BlockBehaviour, BlockFuture, BlockMetadata, OnEntityStepArgs, OnPlaceArgs, OnScheduledTickArgs,
};
use crate::entity::EntityBase;
use crate::entity::r#type::from_type;
use crate::world::World;
use pumpkin_data::potion::Effect;
use pumpkin_data::{
    BlockId, BlockStateId,
    block_properties::{BlockProperties, SculkShriekerLikeProperties},
    effect::StatusEffect,
    entity::EntityType,
    sound::{Sound, SoundCategory},
    world::WorldEvent,
};
use pumpkin_util::math::boundingbox::{BoundingBox, EntityDimensions};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;
use rand::{RngExt, rng};
use uuid::Uuid;

const SHRIEK_TICKS: u8 = 90;
const DARKNESS_RADIUS: f64 = 40.0;
const DARKNESS_DURATION: i32 = 260;

pub struct SculkShriekerBlock;

impl BlockMetadata for SculkShriekerBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::SCULK_SHRIEKER].into()
    }
}

impl SculkShriekerBlock {
    pub async fn try_activate(world: &Arc<World>, pos: &BlockPos) -> bool {
        Self::try_activate_from(world, pos, None).await
    }

    /// Activates a shrieker for a vibration with an optional entity source.
    /// Natural warnings are only accepted when that source resolves to an
    /// active player, matching Java's `tryGetPlayer`/`canReceiveVibration`.
    pub async fn try_activate_from(
        world: &Arc<World>,
        pos: &BlockPos,
        source_entity: Option<Uuid>,
    ) -> bool {
        let block = world.get_block(pos);
        if block.id != BlockId::SCULK_SHRIEKER {
            return false;
        }
        let state = world.get_block_state(pos);
        let mut props = SculkShriekerLikeProperties::from_state_id(state.id, block);

        if props.shrieking {
            return false;
        }

        let source_player = source_entity.and_then(|uuid| world.get_player_by_uuid(uuid));
        if source_entity.is_some() && source_player.is_none() {
            return false;
        }

        // SculkShriekerBlockEntity.tryShriek resets the per-block warning
        // before attempting WardenSpawnTracker.tryWarn.  Keeping this reset
        // here is important when a shrieker is reused after a failed summon:
        // a non-summoning shrieker (or a shrieker in Peaceful) must not carry
        // a stale warning into its next response.
        if let Some(entity) = world.get_block_entity(pos)
            && let Some(shrieker) = entity.as_any().downcast_ref::<SculkShriekerBlockEntity>()
        {
            *shrieker.warning_level.lock().await = 0;
        }

        // Vanilla only consults the shared WardenSpawnTracker when this
        // block can summon and the world allows Warden responses.  Decorative
        // shriekers still activate and play their sound even when
        // `can_summon=false`, and they must not be rejected by another
        // player's cooldown.
        if props.can_summon
            && can_respond(world)
            && let Some(player) = source_player.as_ref()
        {
            if player.gamemode.load() == pumpkin_util::GameMode::Spectator
                || player.living_entity.health.load() <= 0.0
            {
                return false;
            }
            if !warn_nearby_players(world, pos, &player).await {
                return false;
            }
        }

        props.shrieking = true;
        world
            .set_block_state(pos, props.to_state_id(block), BlockFlags::NOTIFY_ALL)
            .await;

        // SculkShriekerBlockEntity.shriek emits both the client-side shriek
        // particle and GameEvent.SHRIEK after the state transition. The latter
        // is observable by Warden vibration listeners and adjacent sensors;
        // omitting it silently breaks the vanilla event chain.
        world.sync_world_event(WorldEvent::ParticlesSculkShriek, *pos, 0);
        // The event dispatcher can route a shriek to another listener which
        // in turn activates a shrieker. Box this finite cascade so the async
        // future remains sized without changing the vanilla ordering.
        Box::pin(world.emit_game_event_from(
            *pos,
            crate::world::game_event::GameEventKind::Shriek,
            source_entity,
        ))
        .await;

        world.play_sound(
            Sound::BlockSculkShriekerShriek,
            SoundCategory::Blocks,
            &pos.to_f64(),
        );

        world.schedule_block_tick(block, *pos, SHRIEK_TICKS, TickPriority::Normal);

        if let Some(entity) = world.get_block_entity(pos)
            && let Some(shrieker) = entity.as_any().downcast_ref::<SculkShriekerBlockEntity>()
        {
            let mut level = shrieker.warning_level.lock().await;
            *level = source_player.as_ref().map_or(*level, |player| {
                player
                    .warden_warning_level
                    .load(std::sync::atomic::Ordering::Relaxed)
            });
        }

        true
    }
}

/// Applies Java's shared warning tracker to all non-spectator players within
/// 16 blocks. A shrieker warning is rejected while any nearby tracker is on
/// its 200-tick cooldown; otherwise the highest warning level is incremented
/// and copied to every nearby player.
async fn warn_nearby_players(
    world: &Arc<World>,
    pos: &BlockPos,
    trigger_player: &Arc<crate::entity::player::Player>,
) -> bool {
    // WardenSpawnTracker.tryWarn refuses the warning entirely while a warden
    // is already within its 48-block check volume.  This is distinct from
    // the later spawn-attempt guard: the warning level and cooldown must not
    // advance in that situation.
    if world
        .get_nearby_entities(pos.to_centered_f64(), 48.0)
        .values()
        .any(|entity| entity.get_entity().entity_type.id == EntityType::WARDEN.id)
    {
        return false;
    }
    // World::get_nearby_players is intentionally a raw spatial query.  The
    // vanilla WardenSpawnTracker filters spectators and dead players out of
    // that list before checking cooldowns; they must not block a warning from
    // a living player who happens to share the room.
    let mut players: Vec<_> = world
        .get_nearby_players(pos.to_centered_f64(), 16.0)
        .into_iter()
        .filter(|player| {
            player.gamemode.load() != pumpkin_util::GameMode::Spectator
                && player.living_entity.health.load() > 0.0
        })
        .collect();
    if !players.iter().any(|player| player == trigger_player) {
        players.push(trigger_player.clone());
    }
    if players.iter().any(|player| {
        player
            .warden_cooldown_ticks
            .load(std::sync::atomic::Ordering::Relaxed)
            > 0
    }) {
        return false;
    }
    let warning_level = players
        .iter()
        .map(|player| {
            player
                .warden_warning_level
                .load(std::sync::atomic::Ordering::Relaxed)
        })
        .max()
        .unwrap_or(0)
        .saturating_add(1)
        .min(4);
    for player in players {
        player
            .warden_ticks_since_warning
            .store(0, std::sync::atomic::Ordering::Relaxed);
        player
            .warden_warning_level
            .store(warning_level, std::sync::atomic::Ordering::Relaxed);
        player
            .warden_cooldown_ticks
            .store(200, std::sync::atomic::Ordering::Relaxed);
    }
    true
}

impl BlockBehaviour for SculkShriekerBlock {
    fn on_entity_step<'a>(&'a self, args: OnEntityStepArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            // Vanilla's stepOn path resolves only a player (or a player in a
            // controlling passenger position) and never lets arbitrary mobs
            // activate a shrieker directly.
            let Some(player) = args.entity.get_player() else {
                return;
            };
            let _ = Self::try_activate_from(
                args.world,
                args.position,
                Some(player.get_entity().entity_uuid),
            )
            .await;
        })
    }

    fn on_place<'a>(&'a self, args: OnPlaceArgs<'a>) -> BlockFuture<'a, BlockStateId> {
        Box::pin(async move {
            let mut props = SculkShriekerLikeProperties::default(args.block);
            props.shrieking = false;
            props.waterlogged = args.replacing.water_source();
            props.to_state_id(args.block)
        })
    }

    fn on_scheduled_tick<'a>(&'a self, args: OnScheduledTickArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let state = args.world.get_block_state(args.position);
            let mut props = SculkShriekerLikeProperties::from_state_id(state.id, args.block);
            if props.shrieking {
                props.shrieking = false;
                args.world
                    .set_block_state(
                        args.position,
                        props.to_state_id(args.block),
                        BlockFlags::NOTIFY_ALL,
                    )
                    .await;

                // Warning, darkness and the optional Warden spawn happen when
                // the 90-tick shriek finishes, not when the vibration arrives.
                // This is observable with chained shriekers and prevents a
                // second activation from racing the first warning.
                if props.can_summon
                    && can_respond(args.world)
                    && let Some(entity) = args.world.get_block_entity(args.position)
                    && let Some(shrieker) =
                        entity.as_any().downcast_ref::<SculkShriekerBlockEntity>()
                {
                    let warning_level = *shrieker.warning_level.lock().await;
                    if warning_level > 0 {
                        let summoned = warning_level >= 4
                            && try_summon_warden(args.world, args.position).await;
                        if summoned {
                            *shrieker.warning_level.lock().await = 0;
                        } else {
                            play_warden_reply_sound(args.world, args.position, warning_level);
                        }
                        apply_darkness(args.world, args.position).await;
                    }
                }
            }
        })
    }
}

async fn apply_darkness(world: &Arc<World>, pos: &BlockPos) {
    let darkness = Effect {
        effect_type: &StatusEffect::DARKNESS,
        duration: DARKNESS_DURATION,
        amplifier: 0,
        ambient: false,
        show_particles: false,
        show_icon: true,
        blend: true,
    };
    for player in world.get_nearby_players(pos.to_centered_f64(), DARKNESS_RADIUS) {
        player.send_effect(darkness.clone()).await;
        player.living_entity.add_effect(darkness.clone()).await;
    }
}

/// SculkShriekerBlockEntity's warning-level response when a Warden is not
/// spawned. Vanilla selects one of three proximity sounds for levels 1..3 and
/// the angry listening sound at level 4. The sound position is randomized in
/// a ±10 block cube around the shrieker and uses the hostile 5.0 volume.
fn play_warden_reply_sound(world: &Arc<World>, pos: &BlockPos, warning_level: i32) {
    let Some(sound) = warden_reply_sound(warning_level) else {
        return;
    };
    let sound_pos = Vector3::new(
        f64::from(pos.0.x + rng().random_range(-10..=10)),
        f64::from(pos.0.y + rng().random_range(-10..=10)),
        f64::from(pos.0.z + rng().random_range(-10..=10)),
    );
    world.play_sound_fine(sound, SoundCategory::Hostile, &sound_pos, 5.0, 1.0);
}

#[must_use]
const fn warden_reply_sound(warning_level: i32) -> Option<Sound> {
    match warning_level {
        1 => Some(Sound::EntityWardenNearbyClose),
        2 => Some(Sound::EntityWardenNearbyCloser),
        3 => Some(Sound::EntityWardenNearbyClosest),
        4.. => Some(Sound::EntityWardenListeningAngry),
        _ => None,
    }
}

async fn try_summon_warden(world: &Arc<World>, pos: &BlockPos) -> bool {
    if !can_respond(world) {
        return false;
    }
    let center = pos.to_centered_f64();
    if world
        .get_nearby_entities(center, 48.0)
        .values()
        .any(|entity| entity.get_entity().entity_type.id == EntityType::WARDEN.id)
    {
        return false;
    }

    for _ in 0..20 {
        // SpawnUtil.trySpawnMob starts at `pos.y + 6`, then walks downward
        // through the complete ±6 vertical window.  It does not just sample
        // one random Y coordinate: that distinction matters when the shrieker
        // is embedded in a tall room or on a ledge.
        let x = pos.0.x + rng().random_range(-5..=5);
        let z = pos.0.z + rng().random_range(-5..=5);
        let mut support = BlockPos::new(x, pos.0.y + 6, z);
        let mut above_state = world.get_block_state(&support);

        for _ in 0..13 {
            support = BlockPos::new(support.0.x, support.0.y - 1, support.0.z);
            let current_state = world.get_block_state(&support);
            let above_pos = support.up();

            // SpawnUtil.ON_TOP_OF_COLLIDER: the cell above the support must
            // have an empty collision shape and the support must expose a full
            // upward face.  A side-solid flag is the generated equivalent of
            // Block.isFaceFull(shape, Direction.UP), unlike is_full_cube which
            // rejects valid top slabs and other full-face supports.
            if above_state.get_block_collision_shapes().next().is_none()
                && current_state.is_side_solid(pumpkin_data::BlockDirection::Up)
            {
                let candidate = above_pos;
                let dimensions = EntityDimensions::new(
                    EntityType::WARDEN.dimension[0],
                    EntityType::WARDEN.dimension[1],
                    EntityType::WARDEN.eye_height,
                );
                let spawn = candidate.to_centered_f64();
                let bounds = BoundingBox::new_from_pos(spawn.x, spawn.y, spawn.z, &dimensions);
                if !world.is_space_empty(bounds) || !world.get_entities_at_box(&bounds).is_empty() {
                    above_state = current_state;
                    continue;
                }

                let warden = from_type(&EntityType::WARDEN, spawn, world, Uuid::new_v4());
                world.spawn_entity(warden).await;
                world.play_sound(Sound::EntityWardenEmerge, SoundCategory::Hostile, &spawn);
                return true;
            }

            above_state = current_state;
            // Keep the X/Z offset for the entire vertical search, just like
            // SpawnUtil's mutable search position.
            support = BlockPos::new(x, support.0.y, z);
        }
    }
    false
}

fn can_respond(world: &Arc<World>) -> bool {
    let level = world.level_info.load();
    level.difficulty != pumpkin_util::Difficulty::Peaceful && level.game_rules.spawn_wardens
}

#[cfg(test)]
mod tests {
    use super::warden_reply_sound;
    use pumpkin_data::sound::Sound;

    #[test]
    fn warning_levels_use_vanilla_warden_reply_sounds() {
        assert_eq!(warden_reply_sound(0), None);
        assert_eq!(warden_reply_sound(1), Some(Sound::EntityWardenNearbyClose));
        assert_eq!(warden_reply_sound(2), Some(Sound::EntityWardenNearbyCloser));
        assert_eq!(
            warden_reply_sound(3),
            Some(Sound::EntityWardenNearbyClosest)
        );
        assert_eq!(
            warden_reply_sound(4),
            Some(Sound::EntityWardenListeningAngry)
        );
        assert_eq!(
            warden_reply_sound(255),
            Some(Sound::EntityWardenListeningAngry)
        );
    }
}
