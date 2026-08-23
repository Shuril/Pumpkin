use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use super::{Controls, Goal, GoalFuture};
use crate::entity::ai::pathfinder::NavigatorGoal;
use crate::entity::mob::{Mob, MobEntity};

/// Vanilla `LongDistancePatrolGoal`: patrolling pillagers walk toward their
/// distant patrol target assigned by the patrol leader.
pub struct LongDistancePatrolGoal {
    goal_control: Controls,
    speed: f64,
}

impl LongDistancePatrolGoal {
    const GIVE_UP_DIST_SQ: f64 = 12.0 * 12.0;
    const REPATH_CHANCE: i32 = 10;

    #[must_use]
    pub fn new(speed: f64) -> Self {
        Self {
            goal_control: Controls::MOVE,
            speed,
        }
    }

    async fn patrol_target(entity: &MobEntity) -> Option<BlockPos> {
        *entity.patrol_target.lock().await
    }

    fn target_center(target: BlockPos) -> Vector3<f64> {
        Vector3::new(
            f64::from(target.0.x) + 0.5,
            f64::from(target.0.y),
            f64::from(target.0.z) + 0.5,
        )
    }
}

impl Goal for LongDistancePatrolGoal {
    fn can_start<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move {
            let Some(target) = Self::patrol_target(&mob.get_mob_entity()).await else {
                return false;
            };
            let pos = mob.get_entity().pos.load();
            if pos.squared_distance_to_vec(&Self::target_center(target)) <= Self::GIVE_UP_DIST_SQ {
                return false;
            }
            let navigator_idle = mob
                .get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_idle
                .load(std::sync::atomic::Ordering::Relaxed);
            navigator_idle && rand::rng().random_range(0..Self::REPATH_CHANCE) == 0
        })
    }

    fn should_continue<'a>(&'a self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move {
            let Some(target) = Self::patrol_target(&mob.get_mob_entity()).await else {
                return false;
            };
            let pos = mob.get_entity().pos.load();
            let far_enough =
                pos.squared_distance_to_vec(&Self::target_center(target)) > Self::GIVE_UP_DIST_SQ;
            let navigating = !mob
                .get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_idle
                .load(std::sync::atomic::Ordering::Relaxed);
            far_enough && navigating
        })
    }

    fn start<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            let Some(target) = Self::patrol_target(&mob.get_mob_entity()).await else {
                return;
            };
            let pos = mob.get_entity().pos.load();
            mob.get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .set_progress(NavigatorGoal::new(
                    pos,
                    Self::target_center(target),
                    self.speed,
                ));
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

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
