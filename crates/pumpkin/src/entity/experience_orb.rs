use core::f32;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use pumpkin_data::entity::EntityType;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::vector3::Vector3;

use crate::{entity::EntityBaseFuture, server::Server, world::World};

use super::{Entity, EntityBase, NBTStorage, NbtFuture, living::LivingEntity, player::Player};

pub struct ExperienceOrbEntity {
    entity: Entity,
    /// Value of one orb represented by this entity.  Vanilla keeps this in
    /// synced entity data and persists it as the `Value` short.
    amount: AtomicU32,
    /// Number of equal-value orbs merged into this entity.  A pickup consumes
    /// exactly one count, just like `ExperienceOrb.playerTouch`.
    count: AtomicU32,
    orb_age: AtomicU32,
}

impl ExperienceOrbEntity {
    pub fn new(entity: Entity, amount: u32) -> Self {
        entity.yaw.store(rand::random::<f32>() * 360.0);
        entity
            .data
            .store(amount.min(i32::MAX as u32) as i32, Ordering::Relaxed);
        Self {
            entity,
            amount: AtomicU32::new(amount),
            count: AtomicU32::new(1),
            orb_age: AtomicU32::new(0),
        }
    }

    pub async fn spawn(world: &Arc<World>, position: Vector3<f64>, amount: u32) {
        let mut amount = amount;
        while amount > 0 {
            let i = Self::round_to_orb_size(amount);
            amount -= i;
            let entity = Entity::new(world.clone(), position, &EntityType::EXPERIENCE_ORB);
            let orb = Arc::new(Self::new(entity, i));
            // ExperienceOrb.awardWithDirection first tries to merge each
            // freshly awarded value into an existing orb.  Waiting for the
            // normal 20-tick scan creates a visible duplicate and changes
            // both pickup timing and the persisted Count field.  The entity
            // id modulo-40 gate is the same gate used by the later scan; the
            // lower-id ownership rule keeps this pre-spawn operation safe
            // when world entity ticks run concurrently.
            if Self::try_merge_into_existing(world, &orb).await {
                continue;
            }
            world.spawn_entity(orb).await;
        }
    }

    const fn round_to_orb_size(value: u32) -> u32 {
        if value >= 2477 {
            2477
        } else if value >= 1237 {
            1237
        } else if value >= 617 {
            617
        } else if value >= 307 {
            307
        } else if value >= 149 {
            149
        } else if value >= 73 {
            73
        } else if value >= 37 {
            37
        } else if value >= 17 {
            17
        } else if value >= 7 {
            7
        } else if value >= 3 {
            3
        } else {
            1
        }
    }
}

impl NBTStorage for ExperienceOrbEntity {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.entity.write_nbt(nbt).await;
            nbt.put_short("Health", 5);
            nbt.put_short(
                "Age",
                self.orb_age.load(Ordering::Relaxed).min(i16::MAX as u32) as i16,
            );
            nbt.put_short(
                "Value",
                self.amount.load(Ordering::Relaxed).min(i16::MAX as u32) as i16,
            );
            nbt.put_int(
                "Count",
                self.count.load(Ordering::Relaxed).min(i32::MAX as u32) as i32,
            );
        })
    }

    fn read_nbt_non_mut<'a>(&'a self, nbt: &'a NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.entity.read_nbt_non_mut(nbt).await;
            let amount = nbt.get_short("Value").unwrap_or(0).max(0) as u32;
            self.amount.store(amount, Ordering::Relaxed);
            self.entity
                .data
                .store(amount.min(i32::MAX as u32) as i32, Ordering::Relaxed);
            self.orb_age.store(
                nbt.get_short("Age").unwrap_or(0).max(0) as u32,
                Ordering::Relaxed,
            );
            self.count.store(
                nbt.get_int("Count").unwrap_or(1).max(1) as u32,
                Ordering::Relaxed,
            );
        })
    }
}

impl EntityBase for ExperienceOrbEntity {
    fn tick<'a>(
        &'a self,
        caller: &'a Arc<dyn EntityBase>,
        server: &'a Server,
    ) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move {
            let entity = &self.entity;
            entity.tick(caller, server).await;
            let bounding_box = entity.bounding_box.load();

            let original_velo = entity.velocity.load();

            let mut velo = original_velo;

            let no_clip = !self
                .entity
                .world
                .load()
                .is_space_empty(bounding_box.expand(-1.0e-7, -1.0e-7, -1.0e-7));
            if entity.touching_water.load(Ordering::Relaxed) && entity.water_height.load() > 0.0 {
                // ExperienceOrb.setUnderwaterMovement.
                velo.x *= 0.99;
                velo.z *= 0.99;
                velo.y = (velo.y + 5.0e-4).min(0.06);
            } else if entity.touching_lava.load(Ordering::Relaxed)
                && entity.lava_height.load() > 0.0
            {
                // Experience orbs use a fresh random impulse while in lava.
                velo = Vector3::new(
                    (rand::random::<f64>() - rand::random::<f64>()) * 0.2,
                    0.2,
                    (rand::random::<f64>() - rand::random::<f64>()) * 0.2,
                );
            } else if !no_clip {
                velo.y -= self.get_gravity();
            }

            entity.velocity.store(velo);

            entity.move_entity(caller, velo).await;

            entity.tick_block_collisions(caller, server).await;

            let age = self.orb_age.fetch_add(1, Ordering::Relaxed) + 1;
            if age >= 6000 {
                self.entity.remove().await;
                return;
            }

            // Vanilla scans for equal-value orbs once every 20 ticks (the
            // first scan is tickCount % 20 == 1), then follows the nearest
            // eligible player within eight blocks every tick.
            if age % 20 == 1 {
                self.scan_for_merges().await;
            }
            self.follow_nearest_player().await;
        })
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn on_player_collision<'a>(&'a self, player: &'a Arc<Player>) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move {
            if player.living_entity.health.load() > 0.0 {
                let mut delay = player.experience_pick_up_delay.lock().await;
                if *delay == 0 {
                    *delay = 2;
                    player.living_entity.pickup(&self.entity, 1);
                    let amount = self.amount.load(Ordering::Relaxed).min(i32::MAX as u32) as i32;
                    let remaining = player.apply_mending_from_xp(amount).await;
                    if remaining > 0 {
                        player.add_experience_points(remaining).await;
                    }
                    let count = self.count.load(Ordering::Relaxed);
                    if count <= 1 {
                        self.entity.remove().await;
                    } else {
                        self.count.fetch_sub(1, Ordering::Relaxed);
                    }
                }
            }
        })
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn as_nbt_storage(&self) -> &dyn NBTStorage {
        self
    }

    fn get_gravity(&self) -> f64 {
        0.03
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ExperienceOrbEntity {
    async fn try_merge_into_existing(world: &Arc<World>, orb: &Arc<Self>) -> bool {
        let amount = orb.amount.load(Ordering::Relaxed);
        let search_box = orb.entity.bounding_box.load().expand(0.5, 0.5, 0.5);
        let entity_id = orb.entity.entity_id;

        for candidate in world.get_entities_at_box(&search_box) {
            let Some(other) = candidate.cast_any().downcast_ref::<Self>() else {
                continue;
            };
            if !can_merge_candidate(
                entity_id,
                other.entity.entity_id,
                amount,
                other.amount.load(Ordering::Relaxed),
                false,
                other.entity.removed.load(Ordering::Relaxed),
            ) {
                continue;
            }

            other.count.fetch_add(1, Ordering::Relaxed);
            // Java resets the surviving orb's age when award() coalesces an
            // orb, preventing an otherwise imminent 6000-tick expiration.
            other.orb_age.store(0, Ordering::Relaxed);
            return true;
        }
        false
    }

    async fn scan_for_merges(&self) {
        if self.entity.removed.load(Ordering::Relaxed) {
            return;
        }

        let world = self.entity.world.load();
        let search_box = self.entity.bounding_box.load().expand(0.5, 0.5, 0.5);
        let amount = self.amount.load(Ordering::Relaxed);
        let self_id = self.entity.entity_id;
        let candidates = world.get_entities_at_box(&search_box);

        for candidate in candidates {
            let Some(other) = candidate.cast_any().downcast_ref::<Self>() else {
                continue;
            };
            let other_id = other.entity.entity_id;
            // Only the lower id performs the merge.  This gives the same
            // deterministic ownership as vanilla's one-sided scan and avoids
            // two concurrent ticks deleting both orbs.
            if !can_merge_candidate(
                self_id,
                other_id,
                amount,
                other.amount.load(Ordering::Relaxed),
                self.entity.removed.load(Ordering::Relaxed),
                other.entity.removed.load(Ordering::Relaxed),
            ) {
                continue;
            }

            let other_count = other.count.load(Ordering::Relaxed);
            self.count
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                    Some(count.saturating_add(other_count))
                })
                .ok();
            self.orb_age
                .fetch_min(other.orb_age.load(Ordering::Relaxed), Ordering::Relaxed);
            other.entity.remove().await;
        }
    }

    async fn follow_nearest_player(&self) {
        if self.entity.removed.load(Ordering::Relaxed) {
            return;
        }
        let world = self.entity.world.load();
        let orb_pos = self.entity.pos.load();
        let search_box = self.entity.bounding_box.load().expand(8.0, 8.0, 8.0);
        let target = world
            .get_players_at_box(&search_box)
            .into_iter()
            .filter(|player| {
                !player.is_spectator()
                    && player.living_entity.health.load() > 0.0
                    && player
                        .get_entity()
                        .pos
                        .load()
                        .squared_distance_to_vec(&orb_pos)
                        <= 64.0
            })
            .min_by(|a, b| {
                let a_distance = a.get_entity().pos.load().squared_distance_to_vec(&orb_pos);
                let b_distance = b.get_entity().pos.load().squared_distance_to_vec(&orb_pos);
                a_distance.total_cmp(&b_distance)
            });

        let Some(player) = target else {
            return;
        };
        let player_pos = player.get_entity().pos.load();
        let delta = Vector3::new(
            player_pos.x - orb_pos.x,
            player_pos.y + (player.get_entity().height() as f64 * 0.5) - orb_pos.y,
            player_pos.z - orb_pos.z,
        );
        let distance_squared = delta.length_squared();
        if distance_squared <= f64::EPSILON {
            return;
        }
        let power = 1.0 - (distance_squared.sqrt() / 8.0);
        let mut velocity = self.entity.velocity.load();
        velocity += delta.normalize() * (power * power * 0.1);
        self.entity.velocity.store(velocity);
    }
}

fn can_merge_candidate(
    self_id: i32,
    other_id: i32,
    self_value: u32,
    other_value: u32,
    self_removed: bool,
    other_removed: bool,
) -> bool {
    other_id > self_id
        && !self_removed
        && !other_removed
        && self_value == other_value
        && (other_id - self_id).rem_euclid(40) == 0
}

#[cfg(test)]
mod tests {
    use super::{ExperienceOrbEntity, can_merge_candidate};

    #[test]
    fn orb_sizes_match_vanilla_thresholds() {
        assert_eq!(ExperienceOrbEntity::round_to_orb_size(1), 1);
        assert_eq!(ExperienceOrbEntity::round_to_orb_size(6), 3);
        assert_eq!(ExperienceOrbEntity::round_to_orb_size(7), 7);
        assert_eq!(ExperienceOrbEntity::round_to_orb_size(2_500), 2_477);
    }

    #[test]
    fn merge_gate_is_one_sided_and_value_sensitive() {
        assert!(can_merge_candidate(10, 50, 7, 7, false, false));
        assert!(!can_merge_candidate(50, 10, 7, 7, false, false));
        assert!(!can_merge_candidate(10, 50, 3, 7, false, false));
        assert!(!can_merge_candidate(10, 51, 7, 7, false, false));
        assert!(!can_merge_candidate(10, 50, 7, 7, true, false));
    }
}
