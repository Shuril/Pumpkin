use super::{Mob, MobEntity};
use crate::entity::ai::goal::break_door::BreakDoorGoal;
use crate::entity::ai::goal::destroy_egg::DestroyEggGoal;
use crate::entity::ai::goal::look_around::RandomLookAroundGoal;
use crate::entity::ai::goal::revenge::RevengeGoal;
use crate::entity::ai::goal::swim::SwimGoal;
use crate::entity::ai::goal::wander_around::WanderAroundGoal;
use crate::entity::ai::goal::zombie_attack::ZombieAttackGoal;
use crate::entity::{
    Entity, EntityBase, EntityBaseFuture, NBTStorage, NbtFuture,
    ai::goal::{active_target::ActiveTargetGoal, look_at_entity::LookAtEntityGoal},
};
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::compound::NbtCompound;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

pub mod drowned;
pub mod husk;
#[allow(clippy::module_inception)]
pub mod zombie;
pub mod zombie_villager;

pub struct ZombieEntityBase {
    pub mob_entity: MobEntity,
    pub can_break_doors: AtomicBool,
}

impl ZombieEntityBase {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let zombie = Self {
            mob_entity,
            can_break_doors: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(zombie);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            goal_selector.add_goal(0, Box::new(SwimGoal::default()));
            goal_selector.add_goal(1, Box::new(BreakDoorGoal::new(mob_weak.clone())));
            goal_selector.add_goal(2, ZombieAttackGoal::new(1.0, false));
            goal_selector.add_goal(4, DestroyEggGoal::new(1.0, 3));
            goal_selector.add_goal(7, Box::new(WanderAroundGoal::new(1.0)));
            goal_selector.add_goal(
                8,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(8, Box::new(RandomLookAroundGoal::default()));

            target_selector.add_goal(1, Box::new(RevengeGoal::new(true)));
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::PLAYER, true),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::VILLAGER, true),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::IRON_GOLEM, true),
            );
            target_selector.add_goal(
                5,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::TURTLE, true),
            );
        };

        mob_arc
    }

    #[must_use]
    pub fn can_break_doors(&self) -> bool {
        self.can_break_doors.load(Ordering::Relaxed)
    }

    pub fn set_can_break_doors(&self, can_break: bool) {
        self.can_break_doors.store(can_break, Ordering::Relaxed);
        let mut navigator = self
            .mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        navigator.set_can_open_doors(can_break);
    }
    pub fn handle_creeper_drop_mob_head<'a>(
        &'a self,
        source: Option<&'a dyn EntityBase>,
        cause: Option<&'a dyn EntityBase>,
    ) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move {
            let entity = &self.mob_entity.living_entity.entity;
            let world = entity.world.load();
            if !world.level_info.load().game_rules.mob_drops {
                return;
            }

            let killer = cause.or(source);
            if let Some(killer) = killer
                && let Some(creeper) = killer.get_mob().and_then(|m| m.get_creeper())
                && creeper.can_drop_mob_head()
            {
                creeper.increase_dropped_mob_heads();
                world
                    .drop_stack(
                        &entity.block_pos.load(),
                        ItemStack::new(1, &Item::ZOMBIE_HEAD),
                    )
                    .await;
            }
        })
    }
}

impl NBTStorage for ZombieEntityBase {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.write_nbt(nbt).await;
            nbt.put_bool("CanBreakDoors", self.can_break_doors());
        })
    }

    fn read_nbt_non_mut<'a>(&'a self, nbt: &'a NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.read_nbt_non_mut(nbt).await;
            if let Some(can_break) = nbt.get_bool("CanBreakDoors") {
                self.set_can_break_doors(can_break);
            }
        })
    }
}

impl Mob for ZombieEntityBase {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn can_break_doors(&self) -> bool {
        self.can_break_doors()
    }

    fn set_can_break_doors(&self, can_break: bool) {
        self.set_can_break_doors(can_break);
    }

    fn mob_drop_custom_death_loot<'a>(
        &'a self,
        _damage_type: DamageType,
        source: Option<&'a dyn EntityBase>,
        cause: Option<&'a dyn EntityBase>,
    ) -> EntityBaseFuture<'a, ()> {
        self.handle_creeper_drop_mob_head(source, cause)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zombie_can_break_doors_flag() {
        let flag = AtomicBool::new(false);
        assert!(!flag.load(Ordering::Relaxed));
        flag.store(true, Ordering::Relaxed);
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn test_zombie_can_break_doors_nbt_serialization() {
        let mut compound = NbtCompound::new();
        compound.put_bool("CanBreakDoors", true);
        assert_eq!(compound.get_bool("CanBreakDoors"), Some(true));

        let flag = AtomicBool::new(false);
        if let Some(can_break) = compound.get_bool("CanBreakDoors") {
            flag.store(can_break, Ordering::Relaxed);
        }
        assert!(flag.load(Ordering::Relaxed));

        let mut roundtrip = NbtCompound::new();
        roundtrip.put_bool("CanBreakDoors", flag.load(Ordering::Relaxed));
        assert_eq!(roundtrip.get_bool("CanBreakDoors"), Some(true));
    }

    #[test]
    fn test_navigator_can_open_doors_toggle() {
        use crate::entity::ai::pathfinder::Navigator;
        let mut navigator = Navigator::default();
        assert!(!navigator.can_open_doors());
        navigator.set_can_open_doors(true);
        assert!(navigator.can_open_doors());
        assert!(navigator.can_pass_doors());
        navigator.set_can_open_doors(false);
        assert!(!navigator.can_open_doors());
    }

    #[test]
    fn test_zombie_head_item_is_valid() {
        assert!(Item::from_id(Item::ZOMBIE_HEAD.id).is_some());
    }
}
