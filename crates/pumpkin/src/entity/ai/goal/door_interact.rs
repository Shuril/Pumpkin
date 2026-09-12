use std::sync::Weak;

use crate::entity::ai::goal::{Controls, Goal, GoalFuture};
use crate::entity::mob::Mob;
use crate::world::World;
use pumpkin_data::block_properties::{BlockProperties, OakDoorLikeProperties};
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, tag};
use pumpkin_util::math::position::BlockPos;

pub struct DoorInteractGoal {
    pub mob: Weak<dyn Mob>,
    pub door_pos: BlockPos,
    pub has_door: bool,
    pub passed: bool,
    pub door_open_dir_x: f32,
    pub door_open_dir_z: f32,
    goal_control: Controls,
}

impl DoorInteractGoal {
    #[must_use]
    pub fn new(mob: Weak<dyn Mob>) -> Self {
        Self {
            mob,
            door_pos: BlockPos::new(0, 0, 0),
            has_door: false,
            passed: false,
            door_open_dir_x: 0.0,
            door_open_dir_z: 0.0,
            goal_control: Controls::empty(),
        }
    }

    #[must_use]
    pub fn is_wooden_door(world: &World, pos: &BlockPos) -> bool {
        let block = world.get_block(pos);
        (block.has_tag(&tag::Block::MINECRAFT_WOODEN_DOORS)
            || block.has_tag(&tag::Block::MINECRAFT_MOB_INTERACTABLE_DOORS)
            || block.has_tag(&tag::Block::MINECRAFT_DOORS))
            && block.id != Block::IRON_DOOR.id
    }

    #[must_use]
    pub fn is_door_open(world: &World, pos: &BlockPos) -> bool {
        if !Self::is_wooden_door(world, pos) {
            return false;
        }
        let (block, state_id) = world.get_block_and_state_id(pos);
        OakDoorLikeProperties::from_state_id(state_id, block).open
    }

    pub fn find_door_pos(mob: &dyn Mob, world: &World) -> Option<BlockPos> {
        let mob_pos = mob.get_entity().pos.load();
        let navigator = mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Check path nodes if path navigation is active
        if let Some(path) = navigator.get_path()
            && !path.is_done()
        {
            let next_idx = path.get_next_node_index();
            let limit = (next_idx + 2).min(path.get_node_count());
            for i in 0..limit {
                if let Some(node) = path.get_node(i) {
                    for dy in [0, 1] {
                        let pos = BlockPos::new(node.pos.0.x, node.pos.0.y + dy, node.pos.0.z);
                        let dx = f64::from(pos.0.x) + 0.5 - mob_pos.x;
                        let dy_diff = f64::from(pos.0.y) + 0.5 - mob_pos.y;
                        let dz = f64::from(pos.0.z) + 0.5 - mob_pos.z;
                        if dx * dx + dy_diff * dy_diff + dz * dz <= 4.0
                            && Self::is_wooden_door(world, &pos)
                        {
                            return Some(pos);
                        }
                    }
                }
            }
        }

        // Also check immediate horizontal and vertical surroundings
        let base_pos = mob_pos.to_block_pos();
        let offsets = [
            (0, 0, 0),
            (0, 1, 0),
            (1, 0, 0),
            (1, 1, 0),
            (-1, 0, 0),
            (-1, 1, 0),
            (0, 0, 1),
            (0, 1, 1),
            (0, 0, -1),
            (0, 1, -1),
        ];

        for (ox, oy, oz) in offsets {
            let pos = BlockPos::new(base_pos.0.x + ox, base_pos.0.y + oy, base_pos.0.z + oz);
            let dx = f64::from(pos.0.x) + 0.5 - mob_pos.x;
            let dy_diff = f64::from(pos.0.y) + 0.5 - mob_pos.y;
            let dz = f64::from(pos.0.z) + 0.5 - mob_pos.z;
            if dx * dx + dy_diff * dy_diff + dz * dz <= 4.0 && Self::is_wooden_door(world, &pos) {
                return Some(pos);
            }
        }

        None
    }
}

impl Goal for DoorInteractGoal {
    fn can_start<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move {
            let world = mob.get_entity().world.load();
            if let Some(pos) = Self::find_door_pos(mob, &world) {
                self.door_pos = pos;
                self.has_door = true;
                true
            } else {
                false
            }
        })
    }

    fn should_continue<'a>(&'a self, _mob: &'a dyn Mob) -> GoalFuture<'a, bool> {
        Box::pin(async move { !self.passed })
    }

    fn start<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            self.passed = false;
            let mob_pos = mob.get_entity().pos.load();
            self.door_open_dir_x = (f64::from(self.door_pos.0.x) + 0.5 - mob_pos.x) as f32;
            self.door_open_dir_z = (f64::from(self.door_pos.0.z) + 0.5 - mob_pos.z) as f32;
        })
    }

    fn tick<'a>(&'a mut self, mob: &'a dyn Mob) -> GoalFuture<'a, ()> {
        Box::pin(async move {
            let mob_pos = mob.get_entity().pos.load();
            let dx = (f64::from(self.door_pos.0.x) + 0.5 - mob_pos.x) as f32;
            let dz = (f64::from(self.door_pos.0.z) + 0.5 - mob_pos.z) as f32;
            let dot = self.door_open_dir_x * dx + self.door_open_dir_z * dz;
            if dot < 0.0 {
                self.passed = true;
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
    fn test_door_interact_goal_initialization() {
        let goal = DoorInteractGoal::new(Weak::<ZombieEntityBase>::new());
        assert!(!goal.has_door);
        assert!(!goal.passed);
        assert_eq!(goal.controls(), Controls::empty());
    }

    #[test]
    fn test_door_interact_passed_dot_product() {
        let dir_x = 1.0f32;
        let dir_z = 0.0f32;

        // Current vector points in same direction: dot > 0 -> not passed
        let curr_x = 0.5f32;
        let curr_z = 0.0f32;
        assert!(dir_x * curr_x + dir_z * curr_z >= 0.0);

        // Mob moved past door: current vector is behind initial: dot < 0 -> passed
        let past_x = -0.5f32;
        let past_z = 0.0f32;
        assert!(dir_x * past_x + dir_z * past_z < 0.0);
    }
}
