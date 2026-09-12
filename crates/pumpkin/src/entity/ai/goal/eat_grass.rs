use super::{Controls, Goal, GoalFuture};
use crate::entity::mob::Mob;
use pumpkin_data::Block;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_world::world::BlockFlags;
use rand::RngExt;

const MAX_TIMER: i32 = 40;

pub struct EatGrassGoal {
    goal_control: Controls,
    timer: i32,
}

impl Default for EatGrassGoal {
    fn default() -> Self {
        Self {
            goal_control: Controls::MOVE | Controls::LOOK | Controls::JUMP,
            timer: 0,
        }
    }
}

impl EatGrassGoal {
    #[must_use]
    pub const fn get_timer(&self) -> i32 {
        self.timer
    }
}

impl Goal for EatGrassGoal {
    fn can_start<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move {
            let entity = &mob.get_mob_entity().living_entity.entity;
            let chance = if entity.age.load(std::sync::atomic::Ordering::Relaxed) < 0 {
                50
            } else {
                1000
            };
            if mob.get_random().random_range(0..chance) != 0 {
                return false;
            }

            let block_pos = entity.block_pos.load();
            let world = entity.world.load();

            let block_at_pos = world.get_block(&block_pos);
            if block_at_pos.has_tag(&tag::Block::MINECRAFT_EDIBLE_FOR_SHEEP) {
                return true;
            }

            let block_below = world.get_block(&block_pos.down());
            block_below.id == Block::GRASS_BLOCK.id
        })
    }

    fn should_continue<'a>(&'a self, _mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move { self.timer > 0 })
    }

    fn start<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            self.timer = MAX_TIMER;
            let entity = &mob.get_mob_entity().living_entity.entity;
            let world = entity.world.load();
            world.send_entity_status(
                entity,
                pumpkin_data::entity::EntityStatus::EatGrass,
                Some(pumpkin_protocol::bedrock::server::actor_event::ActorEventType::EatGrass),
            );
            let mut navigator = mob
                .get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            navigator.stop();
        })
    }

    fn tick<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            self.timer -= 1;

            if self.timer == 4 {
                let entity = &mob.get_mob_entity().living_entity.entity;
                let block_pos = entity.block_pos.load();
                let world = entity.world.load_full();
                let mob_griefing = world.level_info.load().game_rules.mob_griefing;

                let block_at_pos = world.get_block(&block_pos);
                if block_at_pos.has_tag(&tag::Block::MINECRAFT_EDIBLE_FOR_SHEEP) {
                    if mob_griefing {
                        world
                            .set_block_state(
                                &block_pos,
                                Block::AIR.default_state.id,
                                BlockFlags::NOTIFY_ALL,
                            )
                            .await;
                    }
                    mob.on_eating_grass().await;
                } else {
                    let below_pos = block_pos.down();
                    let block_below = world.get_block(&below_pos);
                    if block_below.id == Block::GRASS_BLOCK.id {
                        if mob_griefing {
                            world.sync_world_event(
                                pumpkin_data::world::WorldEvent::ParticlesDestroyBlock,
                                below_pos,
                                i32::from(Block::GRASS_BLOCK.default_state.id.as_u16()),
                            );
                            world
                                .set_block_state(
                                    &below_pos,
                                    Block::DIRT.default_state.id,
                                    BlockFlags::NOTIFY_ALL,
                                )
                                .await;
                        }
                        mob.on_eating_grass().await;
                    }
                }
            }
        })
    }

    fn stop<'a>(&'a mut self, _mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            self.timer = 0;
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
    use super::{Controls, EatGrassGoal, MAX_TIMER};

    #[test]
    fn test_eat_grass_goal_initialization() {
        let goal = EatGrassGoal::default();
        assert_eq!(goal.get_timer(), 0);
        assert_eq!(
            goal.goal_control,
            Controls::MOVE | Controls::LOOK | Controls::JUMP
        );
        assert_eq!(MAX_TIMER, 40);
    }

    #[test]
    fn test_baby_sheep_age_acceleration() {
        // -24000 ticks is baby start age in vanilla
        let age: i32 = -24000;
        let accelerated = (age + 1200).min(0);
        assert_eq!(accelerated, -22800);

        // Near maturity
        let age_near: i32 = -500;
        let accelerated_near = (age_near + 1200).min(0);
        assert_eq!(accelerated_near, 0);
    }
}
