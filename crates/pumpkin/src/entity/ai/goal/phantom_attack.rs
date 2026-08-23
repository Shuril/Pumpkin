use std::sync::Arc;

use pumpkin_util::GameMode;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use super::{Controls, Goal, GoalFuture};
use crate::entity::EntityBase;
use crate::entity::mob::Mob;

const ORBIT_RADIUS: f64 = 16.0;
const ORBIT_HEIGHT_ABOVE_PLAYER: f64 = 16.0;
const ORBIT_SPEED: f64 = 0.6;
const ORBIT_PLAYER_RANGE: f64 = 64.0;
const SWEEP_START_RANGE: f64 = 24.0;
const SWEEP_HIT_RANGE_SQ: f64 = 9.0;
const DIVE_SPEED: f64 = 1.1;
const SWEEP_COOLDOWN_TICKS: i32 = 60;

fn closest_player(mob: &dyn Mob, range: f64) -> Option<Arc<dyn EntityBase>> {
    let world = mob.get_entity().world.load();
    let pos = mob.get_entity().pos.load();
    let player = world.get_closest_player(pos, range)?;
    (player.gamemode.load() != GameMode::Spectator).then_some(player)
}

/// Vanilla `PhantomCircleAroundAnchorGoal`: orbits above the closest player,
/// steering directly with velocity since phantoms are flying mobs.
pub struct PhantomCirclingGoal {
    goal_control: Controls,
    angle: f64,
}

impl Default for PhantomCirclingGoal {
    fn default() -> Self {
        Self {
            goal_control: Controls::MOVE,
            angle: rand::random::<f64>() * std::f64::consts::TAU,
        }
    }
}

impl Goal for PhantomCirclingGoal {
    fn can_start<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move { closest_player(mob, ORBIT_PLAYER_RANGE).is_some() })
    }

    fn should_continue<'a>(&'a self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move { closest_player(mob, ORBIT_PLAYER_RANGE).is_some() })
    }

    fn tick<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            let Some(player) = closest_player(mob, ORBIT_PLAYER_RANGE) else {
                return;
            };
            let player_pos = player.get_entity().pos.load();

            self.angle = (self.angle + 0.15).rem_euclid(std::f64::consts::TAU);
            let orbit_point = Vector3::new(
                player_pos.x + self.angle.cos() * ORBIT_RADIUS,
                player_pos.y + ORBIT_HEIGHT_ABOVE_PLAYER + (self.angle * 2.0).sin() * 3.0,
                player_pos.z + self.angle.sin() * ORBIT_RADIUS,
            );

            let entity = mob.get_entity();
            let pos = entity.pos.load();
            let delta = orbit_point.sub(&pos);
            let horizontal_len = delta.x.hypot(delta.z).max(0.001);
            let mut velocity = Vector3::new(
                delta.x / horizontal_len * ORBIT_SPEED,
                (delta.y * 0.12).clamp(-0.4, 0.4),
                delta.z / horizontal_len * ORBIT_SPEED,
            );
            if delta.length_squared() < 1.0 {
                velocity = Vector3::new(0.0, velocity.y, 0.0);
            }
            entity.set_velocity(velocity);
            entity.set_rotation(f64::atan2(-delta.x, delta.z) as f32, 0.0);
        })
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}

/// Vanilla `PhantomSweepAttackGoal`: dives at the tracked player and bites
/// when close enough.
#[derive(Default)]
pub struct PhantomSweepAttackGoal {
    goal_control: Controls,
    cooldown: i32,
}

impl Goal for PhantomSweepAttackGoal {
    fn can_start<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move {
            if self.cooldown > 0 {
                self.cooldown -= 1;
                return false;
            }
            if rand::rng().random_range(0..20) != 0 {
                return false;
            }
            let Some(target) = closest_player(mob, SWEEP_START_RANGE) else {
                return false;
            };
            let pos = mob.get_entity().pos.load();
            target.get_entity().pos.load().y < pos.y - 4.0
        })
    }

    fn should_continue<'a>(&'a self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move {
            let Some(target) = closest_player(mob, SWEEP_START_RANGE) else {
                return false;
            };
            let pos = mob.get_entity().pos.load();
            target.get_entity().is_alive()
                && target.get_entity().pos.load().squared_distance_to_vec(&pos) > SWEEP_HIT_RANGE_SQ
        })
    }

    fn tick<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            let Some(target) = closest_player(mob, SWEEP_START_RANGE) else {
                return;
            };
            let target_pos = target.get_entity().pos.load();
            let entity = mob.get_entity();
            let pos = entity.pos.load();
            let delta = target_pos.sub(&pos);
            let len = delta.length().max(0.001);
            entity.set_velocity(Vector3::new(
                delta.x / len * DIVE_SPEED,
                delta.y / len * DIVE_SPEED,
                delta.z / len * DIVE_SPEED,
            ));

            if target_pos.squared_distance_to_vec(&pos) <= SWEEP_HIT_RANGE_SQ {
                mob.get_mob_entity().try_attack(mob, target.as_ref()).await;
                self.cooldown = SWEEP_COOLDOWN_TICKS;
                entity.set_velocity(Vector3::new(0.0, 0.2, 0.0));
            }
        })
    }

    fn stop<'a>(&'a mut self, _mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            self.cooldown = SWEEP_COOLDOWN_TICKS;
        })
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
