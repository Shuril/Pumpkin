use super::BlockEntity;
use crate::entity::EntityBase;
use crate::world::World;
use pumpkin_data::Block;
use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityType;
use pumpkin_data::fluid::Fluid;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use rand::RngExt;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::sync::Mutex;
use uuid::Uuid;

const BLOCK_REFRESH_RATE: i64 = 40;
const EFFECT_DURATION: i32 = 13;
const MIN_ACTIVE_SIZE: usize = 16;
const MIN_KILL_SIZE: usize = 42;
const KILL_RANGE: f64 = 8.0;

/// Server-side conduit state.  The frame is deliberately recalculated from
/// world blocks every 40 ticks, just like vanilla; storing only the derived
/// active/target values keeps chunk NBT compatible and avoids stale frames
/// after a player edits the prismarine ring.
pub struct ConduitBlockEntity {
    pub position: BlockPos,
    pub active: Mutex<bool>,
    pub target: Mutex<Option<Uuid>>,
    next_ambient_sound: AtomicI64,
}

impl BlockEntity for ConduitBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        Self {
            position,
            active: Mutex::new(nbt.get_bool("Active").unwrap_or(false)),
            target: Mutex::new(nbt.get_uuid("Target")),
            next_ambient_sound: AtomicI64::new(0),
        }
    }

    fn write_nbt<'a>(
        &'a self,
        nbt: &'a mut NbtCompound,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            nbt.put_bool("Active", *self.active.lock().await);
            if let Some(target) = *self.target.lock().await {
                nbt.put_uuid("Target", target);
            }
        })
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        nbt.put_bool("Active", *self.active.try_lock().ok()?);
        if let Ok(target) = self.target.try_lock()
            && let Some(target) = *target
        {
            nbt.put_uuid("Target", target);
        }
        Some(nbt)
    }

    fn tick<'a>(&'a self, world: &'a Arc<World>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let game_time = world.get_world_age().await;
            if game_time % BLOCK_REFRESH_RATE == 0 {
                let frame_size = self.refresh_frame(world);
                let active = frame_size >= MIN_ACTIVE_SIZE;
                let was_active = {
                    let mut state = self.active.lock().await;
                    let was_active = *state;
                    *state = active;
                    was_active
                };

                if active != was_active {
                    world.play_block_sound(
                        if active {
                            Sound::BlockConduitActivate
                        } else {
                            Sound::BlockConduitDeactivate
                        },
                        SoundCategory::Blocks,
                        self.position,
                    );
                }

                if active {
                    self.apply_effects(world, frame_size).await;
                    if frame_size >= MIN_KILL_SIZE {
                        self.update_and_attack_target(world).await;
                    } else {
                        *self.target.lock().await = None;
                    }
                } else {
                    *self.target.lock().await = None;
                }
            }

            if *self.active.lock().await {
                if game_time % 80 == 0 {
                    world.play_block_sound(
                        Sound::BlockConduitAmbient,
                        SoundCategory::Blocks,
                        self.position,
                    );
                }
                let next = self.next_ambient_sound.load(Ordering::Relaxed);
                if game_time > next {
                    let delay = 60 + i64::from(rand::rng().random_range(0..40));
                    self.next_ambient_sound
                        .store(game_time + delay, Ordering::Relaxed);
                    world.play_block_sound(
                        Sound::BlockConduitAmbientShort,
                        SoundCategory::Blocks,
                        self.position,
                    );
                }
            }
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ConduitBlockEntity {
    pub const ID: &'static str = "minecraft:conduit";

    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            active: Mutex::new(false),
            target: Mutex::new(None),
            next_ambient_sound: AtomicI64::new(0),
        }
    }

    /// Returns the number of valid prismarine blocks on the six axial rings.
    /// The inner 3×3×3 water cube is required before any frame block counts.
    fn refresh_frame(&self, world: &World) -> usize {
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    let pos = self.position.add(x, y, z);
                    if !is_water(world.get_fluid(&pos)) {
                        return 0;
                    }
                }
            }
        }

        let mut count = 0;
        for x in -2..=2 {
            for y in -2..=2 {
                for z in -2..=2 {
                    if is_frame_position(x, y, z)
                        && is_valid_frame_block(world.get_block(&self.position.add(x, y, z)))
                    {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    async fn apply_effects(&self, world: &Arc<World>, frame_size: usize) {
        let range = (frame_size / 7) * 16;
        let center = self.position.to_f64();
        let bounds = BoundingBox::new(center, center.add_raw(1.0, 1.0, 1.0)).expand(
            range as f64,
            world.dimension.height as f64,
            range as f64,
        );
        let effect = pumpkin_data::potion::Effect {
            effect_type: &StatusEffect::CONDUIT_POWER,
            duration: EFFECT_DURATION,
            amplifier: 0,
            ambient: true,
            show_particles: true,
            show_icon: true,
            blend: false,
        };
        for player in world.get_players_at_box(&bounds) {
            let pos = player.get_entity().pos.load();
            if pos.squared_distance_to_vec(&center) > (range as f64) * (range as f64) {
                continue;
            }
            let wet = player.get_entity().touching_water.load(Ordering::Relaxed)
                || world
                    .is_raining_at(&player.get_entity().block_pos.load())
                    .await;
            if wet {
                player.add_effect(effect.clone()).await;
            }
        }
    }

    async fn update_and_attack_target(&self, world: &Arc<World>) {
        let center = self.position.to_f64();
        let current = *self.target.lock().await;
        let target = current.and_then(|uuid| {
            let entity = world.get_entity_by_uuid(uuid)?;
            let living = entity.get_living_entity()?;
            (entity.get_entity().is_alive()
                && living.can_take_damage()
                && entity
                    .get_entity()
                    .pos
                    .load()
                    .squared_distance_to_vec(&center)
                    <= KILL_RANGE * KILL_RANGE)
                .then_some(entity)
        });

        let target = if let Some(target) = target {
            Some(target)
        } else {
            let bounds = BoundingBox::new(center, center.add_raw(1.0, 1.0, 1.0))
                .expand(KILL_RANGE, KILL_RANGE, KILL_RANGE);
            let mut candidates = Vec::new();
            for entity in world.get_entities_at_box(&bounds) {
                let base = entity.get_entity();
                let Some(living) = entity.get_living_entity() else {
                    continue;
                };
                if !is_conduit_enemy(base.entity_type)
                    || !base.is_alive()
                    || !living.can_take_damage()
                    || base.pos.load().squared_distance_to_vec(&center) > KILL_RANGE * KILL_RANGE
                    || !(base.touching_water.load(Ordering::Relaxed)
                        || world.is_raining_at(&base.block_pos.load()).await)
                {
                    continue;
                }
                candidates.push(entity);
            }
            use rand::seq::IndexedRandom;
            candidates.choose(&mut rand::rng()).cloned()
        };

        let Some(target) = target else {
            *self.target.lock().await = None;
            return;
        };
        let target_uuid = target.get_entity().entity_uuid;
        *self.target.lock().await = Some(target_uuid);
        if let Some(living) = target.get_living_entity() {
            world.play_sound(
                Sound::BlockConduitAttackTarget,
                SoundCategory::Blocks,
                &target.get_entity().pos.load(),
            );
            living.damage(target.as_ref(), 4.0, DamageType::MAGIC).await;
        }
    }
}

fn is_water(fluid: &Fluid) -> bool {
    fluid.id == Fluid::WATER.id || fluid.id == Fluid::FLOWING_WATER.id
}

/// Java's conduit target predicate uses the `Enemy` marker interface rather
/// than the broad mob-category value.  Pumpkin's generated registry does not
/// carry marker traits, so keep the registry mapping explicit here.  This is
/// deliberately an allow-list: projectiles, vehicles, undead mounts and
/// future non-hostile `MONSTER` category entries must not become conduit
/// targets merely because their category happens to match.
fn is_conduit_enemy(entity_type: &EntityType) -> bool {
    matches!(
        entity_type.id,
        id if id == EntityType::BLAZE.id
            || id == EntityType::BOGGED.id
            || id == EntityType::BREEZE.id
            || id == EntityType::CREAKING.id
            || id == EntityType::CREEPER.id
            || id == EntityType::DROWNED.id
            || id == EntityType::ELDER_GUARDIAN.id
            || id == EntityType::ENDER_DRAGON.id
            || id == EntityType::ENDERMAN.id
            || id == EntityType::ENDERMITE.id
            || id == EntityType::EVOKER.id
            || id == EntityType::GHAST.id
            || id == EntityType::GUARDIAN.id
            || id == EntityType::HOGLIN.id
            || id == EntityType::HUSK.id
            || id == EntityType::MAGMA_CUBE.id
            || id == EntityType::PHANTOM.id
            || id == EntityType::PIGLIN.id
            || id == EntityType::PIGLIN_BRUTE.id
            || id == EntityType::PILLAGER.id
            || id == EntityType::RAVAGER.id
            || id == EntityType::SHULKER.id
            || id == EntityType::SILVERFISH.id
            || id == EntityType::SKELETON.id
            || id == EntityType::SLIME.id
            || id == EntityType::SPIDER.id
            || id == EntityType::STRAY.id
            || id == EntityType::VEX.id
            || id == EntityType::VINDICATOR.id
            || id == EntityType::WARDEN.id
            || id == EntityType::WITCH.id
            || id == EntityType::WITHER.id
            || id == EntityType::WITHER_SKELETON.id
            || id == EntityType::ZOGLIN.id
            || id == EntityType::ZOMBIE.id
            || id == EntityType::ZOMBIE_VILLAGER.id
            || id == EntityType::ZOMBIFIED_PIGLIN.id
    )
}

fn is_valid_frame_block(block: &Block) -> bool {
    block == &Block::PRISMARINE
        || block == &Block::PRISMARINE_BRICKS
        || block == &Block::SEA_LANTERN
        || block == &Block::DARK_PRISMARINE
}

fn is_frame_position(x: i32, y: i32, z: i32) -> bool {
    let ax = x.abs();
    let ay = y.abs();
    let az = z.abs();
    (ax > 1 || ay > 1 || az > 1)
        && ((x == 0 && (ay == 2 || az == 2))
            || (y == 0 && (ax == 2 || az == 2))
            || (z == 0 && (ax == 2 || ay == 2)))
}

#[cfg(test)]
mod tests {
    use super::{is_conduit_enemy, is_frame_position};
    use pumpkin_data::entity::EntityType;

    #[test]
    fn conduit_frame_has_vanilla_axial_ring_size() {
        let count = (-2..=2)
            .flat_map(|x| (-2..=2).flat_map(move |y| (-2..=2).map(move |z| (x, y, z))))
            .filter(|&(x, y, z)| is_frame_position(x, y, z))
            .count();
        assert_eq!(count, 42);
        assert!(!is_frame_position(0, 0, 0));
        assert!(!is_frame_position(2, 2, 2));
    }

    #[test]
    fn conduit_uses_enemy_marker_semantics_not_only_monster_category() {
        assert!(is_conduit_enemy(&EntityType::CREEPER));
        assert!(is_conduit_enemy(&EntityType::BOGGED));
        assert!(is_conduit_enemy(&EntityType::CREAKING));
        assert!(!is_conduit_enemy(&EntityType::ZOMBIE_HORSE));
        assert!(!is_conduit_enemy(&EntityType::SHULKER_BULLET));
    }
}
