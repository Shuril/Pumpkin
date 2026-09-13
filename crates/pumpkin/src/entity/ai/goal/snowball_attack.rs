use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::{Sound, SoundCategory};
use rand::RngExt;
use std::sync::Arc;

use crate::entity::ai::goal::{Controls, Goal, GoalFuture};
use crate::entity::ai::pathfinder::NavigatorGoal;
use crate::entity::mob::Mob;
use crate::entity::projectile::snowball::SnowballEntity;
use crate::entity::{Entity, EntityBase};

/// Ranged attack goal used by Snow Golems to throw snowballs at hostile mobs.
/// Mirrors vanilla `RangedAttackGoal` on `SnowGolem`.
pub struct SnowballAttackGoal {
    goal_control: Controls,
    speed: f64,
    attack_interval: i32,
    squared_range: f64,
    cooldown: i32,
}

impl SnowballAttackGoal {
    const SNOWBALL_SPEED: f64 = 1.6;
    const DIVERGENCE: f64 = 12.0;

    #[must_use]
    pub fn new(speed: f64, attack_interval: i32, range: f32) -> Self {
        Self {
            goal_control: Controls::MOVE | Controls::LOOK,
            speed,
            attack_interval,
            squared_range: f64::from(range * range),
            cooldown: 0,
        }
    }

    async fn shoot(mob: &dyn Mob, target: &Arc<dyn EntityBase>) {
        let entity = mob.get_entity();
        let world = entity.world.load();
        let mob_pos = entity.pos.load();

        let snowball_entity = Entity::new(world.clone(), mob_pos, &EntityType::SNOWBALL);
        let snowball = SnowballEntity::new_shot(snowball_entity, entity);

        let target_entity = target.get_entity();
        let target_pos = target_entity.pos.load();

        let dx = target_pos.x - mob_pos.x;
        let eye_y = mob_pos.y + f64::from(entity.height()) * 0.85;
        let dy =
            (target_pos.y + f64::from(target_entity.entity_dimension.load().height) * 0.5) - eye_y;
        let dz = target_pos.z - mob_pos.z;
        let horizontal_distance = dx.hypot(dz);

        snowball.thrown.set_velocity(
            dx,
            horizontal_distance.mul_add(0.2, dy),
            dz,
            Self::SNOWBALL_SPEED,
            Self::DIVERGENCE,
        );

        let pitch = 0.4 / (rand::rng().random_range(0.0..0.4) + 0.8);
        world.play_sound_fine(
            Sound::EntitySnowGolemShoot,
            SoundCategory::Neutral,
            &mob_pos,
            1.0,
            pitch,
        );

        let snowball: Arc<dyn EntityBase> = Arc::new(snowball);
        world.spawn_entity(snowball).await;
    }
}

impl Goal for SnowballAttackGoal {
    fn can_start<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move {
            let target = mob.get_mob_entity().target.lock().await.clone();
            let Some(target) = target else {
                return false;
            };
            target.get_entity().is_alive()
        })
    }

    fn should_continue<'a>(&'a self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move {
            let target = mob.get_mob_entity().target.lock().await.clone();
            let Some(target) = target else {
                return false;
            };
            target.get_entity().is_alive()
        })
    }

    fn start<'a>(&'a mut self, _mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            self.cooldown = 0;
        })
    }

    fn stop<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            mob.get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .stop();
        })
    }

    fn tick<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            let target = mob.get_mob_entity().target.lock().await.clone();
            let Some(target) = target else {
                return;
            };

            let mob_pos = mob.get_entity().pos.load();
            let target_pos = target.get_entity().pos.load();
            let distance_sq = mob_pos.squared_distance_to_vec(&target_pos);

            mob.get_mob_entity()
                .look_control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .look_at_entity_with_range(&target, 30.0, 30.0);

            {
                let mut navigator = mob
                    .get_mob_entity()
                    .navigator
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if distance_sq > self.squared_range {
                    navigator.set_progress(NavigatorGoal {
                        current_progress: mob_pos,
                        destination: target_pos,
                        speed: self.speed,
                    });
                } else {
                    navigator.stop();
                }
            }

            self.cooldown -= 1;
            if self.cooldown <= 0 && distance_sq <= self.squared_range {
                Self::shoot(mob, &target).await;
                self.cooldown = self.attack_interval;
            }
        })
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snowball_attack_goal_initialization() {
        let goal = SnowballAttackGoal::new(1.25, 20, 10.0);
        assert_eq!(goal.speed, 1.25);
        assert_eq!(goal.attack_interval, 20);
        assert_eq!(goal.squared_range, 100.0);
    }
}
