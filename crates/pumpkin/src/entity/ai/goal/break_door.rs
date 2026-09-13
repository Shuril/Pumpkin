use std::sync::Weak;

use crate::entity::ai::goal::door_interact::DoorInteractGoal;
use crate::entity::ai::goal::{Controls, Goal, GoalFuture};
use crate::entity::mob::Mob;
use crate::world::BlockBreakingProgress;
use pumpkin_data::block_properties::{BlockProperties, DoubleBlockHalf, OakDoorLikeProperties};
use pumpkin_data::tag;
use pumpkin_data::tag::Taggable;
use pumpkin_data::world::WorldEvent;
use pumpkin_util::Difficulty;
use pumpkin_world::world::BlockFlags;

pub struct BreakDoorGoal {
    pub mob: Weak<dyn Mob>,
    pub door_interact: DoorInteractGoal,
    pub break_progress: u32,
    pub last_break_progress: i32,
    pub door_break_time: u32,
    goal_control: Controls,
}

impl BreakDoorGoal {
    #[must_use]
    pub fn new(mob: Weak<dyn Mob>) -> Self {
        Self {
            mob: mob.clone(),
            door_interact: DoorInteractGoal::new(mob),
            break_progress: 0,
            last_break_progress: -1,
            door_break_time: 240,
            goal_control: Controls::MOVE,
        }
    }

    #[must_use]
    pub fn with_break_time(mob: Weak<dyn Mob>, door_break_time: u32) -> Self {
        Self {
            mob: mob.clone(),
            door_interact: DoorInteractGoal::new(mob),
            break_progress: 0,
            last_break_progress: -1,
            door_break_time,
            goal_control: Controls::MOVE,
        }
    }

    #[must_use]
    pub fn calculate_break_stage(break_progress: u32, door_break_time: u32) -> i32 {
        if door_break_time == 0 {
            return 9;
        }
        let stage = ((break_progress as f32 / door_break_time as f32) * 10.0) as i32;
        stage.clamp(0, 9)
    }
}

impl Goal for BreakDoorGoal {
    fn can_start<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move {
            if !mob.can_break_doors() {
                return false;
            }
            let world = mob.get_entity().world.load();
            if world.level_info.load().difficulty != Difficulty::Hard {
                return false;
            }
            if !world.level_info.load().game_rules.mob_griefing {
                return false;
            }

            if let Some(pos) = DoorInteractGoal::find_door_pos(mob, &world) {
                if !DoorInteractGoal::is_door_open(&world, &pos) {
                    self.door_interact.door_pos = pos;
                    self.door_interact.has_door = true;
                    return true;
                }
            }
            false
        })
    }

    fn should_continue<'a>(&'a self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move {
            if self.break_progress > self.door_break_time {
                return false;
            }
            let world = mob.get_entity().world.load();
            if world.level_info.load().difficulty != Difficulty::Hard {
                return false;
            }
            if !world.level_info.load().game_rules.mob_griefing {
                return false;
            }
            if !DoorInteractGoal::is_wooden_door(&world, &self.door_interact.door_pos) {
                return false;
            }
            if DoorInteractGoal::is_door_open(&world, &self.door_interact.door_pos) {
                return false;
            }

            let mob_pos = mob.get_entity().pos.load();
            let dx = f64::from(self.door_interact.door_pos.0.x) + 0.5 - mob_pos.x;
            let dy = f64::from(self.door_interact.door_pos.0.y) + 0.5 - mob_pos.y;
            let dz = f64::from(self.door_interact.door_pos.0.z) + 0.5 - mob_pos.z;
            dx * dx + dy * dy + dz * dz <= 4.0
        })
    }

    fn start<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            self.break_progress = 0;
            self.last_break_progress = -1;
            let mob_pos = mob.get_entity().pos.load();
            self.door_interact.passed = false;
            self.door_interact.door_open_dir_x =
                (f64::from(self.door_interact.door_pos.0.x) + 0.5 - mob_pos.x) as f32;
            self.door_interact.door_open_dir_z =
                (f64::from(self.door_interact.door_pos.0.z) + 0.5 - mob_pos.z) as f32;
        })
    }

    fn stop<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            let world = mob.get_entity().world.load();
            world
                .set_block_breaking(
                    &mob.get_mob_entity().living_entity.entity,
                    self.door_interact.door_pos,
                    BlockBreakingProgress::Stop,
                )
                .await;
            self.door_interact.has_door = false;
            self.break_progress = 0;
            self.last_break_progress = -1;
        })
    }

    fn tick<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            let world = mob.get_entity().world.load_full();
            let door_pos = self.door_interact.door_pos;
            let mob_pos = mob.get_entity().pos.load();

            // Orient mob to face the door center
            let dx = f64::from(door_pos.0.x) + 0.5 - mob_pos.x;
            let dz = f64::from(door_pos.0.z) + 0.5 - mob_pos.z;
            let target_yaw =
                pumpkin_util::math::wrap_degrees((dz.atan2(dx) as f32).to_degrees() - 90.0);
            mob.get_entity().yaw.store(target_yaw);
            mob.get_entity().head_yaw.store(target_yaw);

            // Play knocking/banging sound and swing hand periodically
            if self.break_progress.is_multiple_of(20) {
                world.sync_world_event(WorldEvent::SoundZombieWoodenDoor, door_pos, 0);
                mob.get_mob_entity().living_entity.swing_hand().await;
            }

            self.break_progress += 1;
            let stage = Self::calculate_break_stage(self.break_progress, self.door_break_time);
            if stage != self.last_break_progress {
                world
                    .set_block_breaking(
                        &mob.get_mob_entity().living_entity.entity,
                        door_pos,
                        BlockBreakingProgress::Update { stage, speed: None },
                    )
                    .await;
                self.last_break_progress = stage;
            }

            if self.break_progress >= self.door_break_time
                && world.level_info.load().difficulty == Difficulty::Hard
            {
                let (block, state_id) = world.get_block_and_state_id(&door_pos);
                let door_props = OakDoorLikeProperties::from_state_id(state_id, block);

                // Destroy primary door block
                world
                    .break_block(&door_pos, None, BlockFlags::SKIP_DROPS)
                    .await;

                // Also destroy counterpart door half (upper/lower)
                let other_half = match door_props.half {
                    DoubleBlockHalf::Upper => pumpkin_data::BlockDirection::Down,
                    DoubleBlockHalf::Lower => pumpkin_data::BlockDirection::Up,
                };
                let other_pos = door_pos.offset(other_half.to_offset());
                if world
                    .get_block(&other_pos)
                    .has_tag(&tag::Block::MINECRAFT_DOORS)
                {
                    world
                        .break_block(&other_pos, None, BlockFlags::SKIP_DROPS)
                        .await;
                }

                world.sync_world_event(WorldEvent::SoundZombieDoorCrash, door_pos, 0);
                world.sync_world_event(
                    WorldEvent::ParticlesDestroyBlock,
                    door_pos,
                    i32::from(state_id.as_u16()),
                );
            }
        })
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::mob::zombie::ZombieEntityBase;

    #[test]
    fn test_break_door_goal_initialization() {
        let goal = BreakDoorGoal::new(Weak::<ZombieEntityBase>::new());
        assert_eq!(goal.break_progress, 0);
        assert_eq!(goal.last_break_progress, -1);
        assert_eq!(goal.door_break_time, 240);
        assert_eq!(goal.controls(), Controls::MOVE);
    }

    #[test]
    fn test_break_progress_stage_calculation() {
        assert_eq!(BreakDoorGoal::calculate_break_stage(0, 240), 0);
        assert_eq!(BreakDoorGoal::calculate_break_stage(24, 240), 1);
        assert_eq!(BreakDoorGoal::calculate_break_stage(120, 240), 5);
        assert_eq!(BreakDoorGoal::calculate_break_stage(240, 240), 9);
        assert_eq!(BreakDoorGoal::calculate_break_stage(300, 240), 9);
    }
}
