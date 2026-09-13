use std::sync::Arc;

use pumpkin_data::entity::{EntityStatus, EntityType};

use super::{Controls, Goal, GoalFuture};
use crate::entity::{EntityBase, mob::Mob};

pub struct OfferFlowerGoal {
    timer: i32,
    target: Option<Arc<dyn EntityBase>>,
}

impl OfferFlowerGoal {
    #[must_use]
    pub fn new() -> Box<Self> {
        Box::new(Self {
            timer: 0,
            target: None,
        })
    }
}

impl Goal for OfferFlowerGoal {
    fn can_start<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async {
            let entity = &mob.get_mob_entity().living_entity.entity;
            let world = entity.world.load();
            if !world.is_day_time() {
                return false;
            }

            if rand::random::<u16>() % 400 != 0 {
                return false;
            }

            let my_pos = entity.pos.load();
            let villagers = world.get_nearby_entities(my_pos, 6.0);
            for (_uuid, candidate) in villagers {
                if candidate.get_entity().entity_type == &EntityType::VILLAGER
                    && candidate.get_entity().age.load(std::sync::atomic::Ordering::Relaxed) < 0
                    && candidate.get_entity().is_alive()
                {
                    self.target = Some(candidate);
                    return true;
                }
            }
            false
        })
    }

    fn should_continue<'a>(&'a self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async {
            let Some(target) = &self.target else {
                return false;
            };

            if self.timer <= 0 || !target.get_entity().is_alive() {
                return false;
            }

            let my_pos = mob.get_mob_entity().living_entity.entity.pos.load();
            let target_pos = target.get_entity().pos.load();
            my_pos.squared_distance_to_vec(&target_pos) <= 36.0
        })
    }

    fn start<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async {
            self.timer = 400;
            let entity = &mob.get_mob_entity().living_entity.entity;
            entity.world.load().send_entity_status(
                entity,
                EntityStatus::OfferFlower,
                None,
            );
        })
    }

    fn stop<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async {
            self.timer = 0;
            self.target = None;
            let entity = &mob.get_mob_entity().living_entity.entity;
            entity.world.load().send_entity_status(
                entity,
                EntityStatus::StopOfferFlower,
                None,
            );
        })
    }

    fn tick<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async {
            if let Some(target) = &self.target {
                let target_pos = target.get_entity().get_eye_pos();
                let mut look_control = mob
                    .get_mob_entity()
                    .look_control
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                look_control.look_at_with_range(
                    target_pos.x,
                    target_pos.y,
                    target_pos.z,
                    30.0,
                    30.0,
                );
            }
            self.timer -= 1;
        })
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}
