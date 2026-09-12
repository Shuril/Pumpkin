use std::sync::Weak;

use pumpkin_data::data_component_impl::EquipmentSlot;

use super::{Controls, Goal, GoalFuture};
use crate::entity::mob::Mob;

pub struct RestrictSunGoal {
    mob: Weak<dyn Mob>,
}

impl RestrictSunGoal {
    #[must_use]
    pub fn new(mob: Weak<dyn Mob>) -> Self {
        Self { mob }
    }

    async fn is_applicable(mob: &dyn Mob) -> bool {
        let mob_entity = mob.get_mob_entity();
        let living_entity = &mob_entity.living_entity;
        let world = living_entity.entity.world.load();
        if !world.is_day_time() {
            return false;
        }
        let equipment = living_entity.entity_equipment.lock().await;
        equipment.get(&EquipmentSlot::HEAD).is_empty()
    }
}

impl Goal for RestrictSunGoal {
    fn can_start<'a>(&'a mut self, _mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move {
            let Some(mob) = self.mob.upgrade() else {
                return false;
            };
            Self::is_applicable(&*mob).await
        })
    }

    fn should_continue<'a>(&'a self, _mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move {
            let Some(mob) = self.mob.upgrade() else {
                return false;
            };
            Self::is_applicable(&*mob).await
        })
    }

    fn start<'a>(&'a mut self, _mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            let Some(mob) = self.mob.upgrade() else {
                return;
            };
            let navigator = mob
                .get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            navigator.set_avoid_sun(true);
        })
    }

    fn stop<'a>(&'a mut self, _mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            let Some(mob) = self.mob.upgrade() else {
                return;
            };
            let navigator = mob
                .get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            navigator.set_avoid_sun(false);
        })
    }

    fn controls(&self) -> Controls {
        Controls::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::ai::pathfinder::Navigator;

    #[test]
    fn test_restrict_sun_goal_initialization() {
        let goal = RestrictSunGoal::new(
            Weak::<crate::entity::mob::skeleton::SkeletonEntityBase>::new(),
        );
        assert_eq!(goal.controls(), Controls::empty());
    }

    #[test]
    fn test_navigator_avoid_sun_toggle() {
        let navigator = Navigator::default();
        assert!(!navigator.avoid_sun());
        navigator.set_avoid_sun(true);
        assert!(navigator.avoid_sun());
        navigator.set_avoid_sun(false);
        assert!(!navigator.avoid_sun());
    }
}
