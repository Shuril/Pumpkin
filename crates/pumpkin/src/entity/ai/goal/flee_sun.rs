use std::sync::Weak;

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use rand::RngExt;

use super::{Controls, Goal, GoalFuture};
use crate::entity::{ai::pathfinder::NavigatorGoal, mob::Mob};

pub struct FleeSunGoal {
    mob: Weak<dyn Mob>,
    pub speed: f32,
    goal_control: Controls,
    target: Option<Vector3<f64>>,
}

impl FleeSunGoal {
    #[must_use]
    pub fn new(mob: Weak<dyn Mob>, speed: f32) -> Self {
        Self {
            mob,
            speed,
            goal_control: Controls::MOVE,
            target: None,
        }
    }

    #[must_use]
    pub fn find_hide_pos<F>(
        pos: BlockPos,
        mut rng: impl rand::Rng,
        can_see_sky: F,
    ) -> Option<Vector3<f64>>
    where
        F: Fn(&BlockPos) -> bool,
    {
        for _ in 0..10 {
            let dx = rng.random_range(-10..=10);
            let dy = rng.random_range(-3..=3);
            let dz = rng.random_range(-10..=10);
            let candidate_pos = BlockPos::new(pos.0.x + dx, pos.0.y + dy, pos.0.z + dz);
            if !can_see_sky(&candidate_pos) {
                return Some(Vector3::new(
                    f64::from(candidate_pos.0.x) + 0.5,
                    f64::from(candidate_pos.0.y),
                    f64::from(candidate_pos.0.z) + 0.5,
                ));
            }
        }
        None
    }
}

impl Goal for FleeSunGoal {
    fn can_start<'a>(&'a mut self, _mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move {
            let Some(mob) = self.mob.upgrade() else {
                return false;
            };
            let mob_entity = mob.get_mob_entity();

            if mob_entity.target.lock().await.is_some() {
                return false;
            }

            if !mob.is_on_fire() {
                return false;
            }

            let living_entity = &mob_entity.living_entity;
            let world = living_entity.entity.world.load();

            if !world.is_day_time() {
                return false;
            }

            let pos = living_entity.entity.block_pos.load();
            if !world.can_see_sky(&pos) {
                return false;
            }

            let equipment = living_entity.entity_equipment.lock().await;
            if !equipment.get(&EquipmentSlot::HEAD).is_empty() {
                return false;
            }

            let rng = mob.get_random();
            let hide_pos = Self::find_hide_pos(pos, rng, |p| world.can_see_sky(p));
            if let Some(target) = hide_pos {
                self.target = Some(target);
                true
            } else {
                false
            }
        })
    }

    fn should_continue<'a>(&'a self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move {
            let navigator = mob
                .get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            !navigator.is_idle()
        })
    }

    fn start<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            if let Some(target) = self.target {
                let mob_arc = self.mob.upgrade();
                let mob_ref: &dyn Mob = mob_arc.as_deref().unwrap_or(mob);
                let pos = mob_ref.get_mob_entity().living_entity.entity.pos.load();
                let mut navigator = mob_ref
                    .get_mob_entity()
                    .navigator
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                navigator.set_progress(NavigatorGoal::new(
                    pos,
                    target,
                    f64::from(self.speed),
                ));
            }
        })
    }

    fn stop<'a>(&'a mut self, _mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            self.target = None;
        })
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flee_sun_goal_initialization() {
        let goal = FleeSunGoal::new(
            Weak::<crate::entity::mob::skeleton::SkeletonEntityBase>::new(),
            1.0,
        );
        assert_eq!(goal.controls(), Controls::MOVE);
        assert!((goal.speed - 1.0).abs() < f32::EPSILON);
        assert!(goal.target.is_none());
    }

    #[test]
    fn test_find_hide_pos_all_sky_visible() {
        let start_pos = BlockPos::new(0, 64, 0);
        let rng = rand::rng();
        let target = FleeSunGoal::find_hide_pos(start_pos, rng, |_| true);
        assert!(target.is_none());
    }

    #[test]
    fn test_find_hide_pos_safe_spot_found() {
        let start_pos = BlockPos::new(0, 64, 0);
        let rng = rand::rng();
        let target = FleeSunGoal::find_hide_pos(start_pos, rng, |_| false);
        assert!(target.is_some());
        let pos = target.unwrap();
        assert!((pos.x - 0.5).abs() <= 10.0);
        assert!((pos.y - 64.0).abs() <= 3.0);
        assert!((pos.z - 0.5).abs() <= 10.0);
    }
}
