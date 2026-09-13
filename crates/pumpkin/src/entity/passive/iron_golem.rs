use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use pumpkin_data::entity::{EntityStatus, EntityType};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::meta_data_type::MetaDataType;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tracked_data::TrackedData;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::java::client::play::Metadata;

use crate::entity::{
    Entity, EntityBase, EntityBaseFuture, NBTStorage, NbtFuture,
    ai::goal::{
        active_target::ActiveTargetGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, melee_attack::MeleeAttackGoal,
        offer_flower::OfferFlowerGoal, revenge::RevengeGoal, wander_around::WanderAroundGoal,
    },
    living::LivingEntity,
    mob::{Mob, MobEntity},
    player::Player,
};

/// Represents the crack level of an Iron Golem based on health fraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Crackiness {
    None = 0,
    Low = 1,
    Medium = 2,
    High = 3,
}

impl Crackiness {
    #[must_use]
    pub fn from_fraction(fraction: f32) -> Self {
        if fraction < 0.25 {
            Self::High
        } else if fraction < 0.5 {
            Self::Medium
        } else if fraction < 0.75 {
            Self::Low
        } else {
            Self::None
        }
    }

    #[must_use]
    pub fn from_health(health: f32, max_health: f32) -> Self {
        if max_health <= 0.0 {
            Self::None
        } else {
            Self::from_fraction(health / max_health)
        }
    }
}

/// Represents an Iron Golem, a powerful neutral mob that protects villagers and players.
///
/// Wiki: <https://minecraft.wiki/w/Iron_Golem>
pub struct IronGolemEntity {
    pub mob_entity: MobEntity,
    pub is_player_created: AtomicBool,
    pub offer_flower_ticks: AtomicI32,
}

impl IronGolemEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let iron_golem = Self {
            mob_entity,
            is_player_created: AtomicBool::new(false),
            offer_flower_ticks: AtomicI32::new(0),
        };
        let mob_arc = Arc::new(iron_golem);
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

            goal_selector.add_goal(1, Box::new(MeleeAttackGoal::new(1.0, true)));
            goal_selector.add_goal(5, OfferFlowerGoal::new());
            goal_selector.add_goal(6, Box::new(WanderAroundGoal::new(0.6)));
            goal_selector.add_goal(
                7,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(8, Box::new(RandomLookAroundGoal::default()));

            target_selector.add_goal(1, Box::new(RevengeGoal::new(true)));
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::ZOMBIE, true),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::SKELETON, true),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::SPIDER, true),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::PILLAGER, true),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::VINDICATOR, true),
            );
        };

        mob_arc
    }

    pub fn is_player_created(&self) -> bool {
        self.is_player_created.load(Ordering::Relaxed)
    }

    pub fn set_player_created(&self, player_created: bool) {
        self.is_player_created
            .store(player_created, Ordering::Relaxed);
        let flags: u8 = if player_created { 1 } else { 0 };
        self.mob_entity.living_entity.entity.send_meta_data(
            &[Metadata::new(
                TrackedData::IRON_GOLEM_FLAGS,
                MetaDataType::BYTE,
                flags,
            )],
            None,
        );
    }

    pub fn offer_flower(&self, offer: bool) {
        if offer {
            self.offer_flower_ticks.store(400, Ordering::Relaxed);
            let entity = &self.mob_entity.living_entity.entity;
            entity.world.load().send_entity_status(
                entity,
                EntityStatus::OfferFlower,
                None,
            );
        } else {
            self.offer_flower_ticks.store(0, Ordering::Relaxed);
            let entity = &self.mob_entity.living_entity.entity;
            entity.world.load().send_entity_status(
                entity,
                EntityStatus::StopOfferFlower,
                None,
            );
        }
    }

    pub fn crackiness(&self) -> Crackiness {
        Crackiness::from_health(
            self.mob_entity.living_entity.health.load(),
            self.mob_entity.living_entity.get_max_health(),
        )
    }
}

impl NBTStorage for IronGolemEntity {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.write_nbt(nbt).await;
            nbt.put_bool("PlayerCreated", self.is_player_created());
        })
    }

    fn read_nbt_non_mut<'a>(&'a self, nbt: &'a NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.read_nbt_non_mut(nbt).await;
            let player_created = nbt.get_byte("PlayerCreated").map_or_else(
                || nbt.get_bool("PlayerCreated").unwrap_or(false),
                |b| b != 0,
            );
            self.is_player_created
                .store(player_created, Ordering::Relaxed);
        })
    }
}

impl Mob for IronGolemEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_init_data_tracker(&self) -> EntityBaseFuture<'_, ()> {
        Box::pin(async move {
            let flags: u8 = if self.is_player_created() { 1 } else { 0 };
            self.mob_entity.living_entity.entity.send_meta_data(
                &[Metadata::new(
                    TrackedData::IRON_GOLEM_FLAGS,
                    MetaDataType::BYTE,
                    flags,
                )],
                None,
            );
        })
    }

    fn mob_interact<'a>(
        &'a self,
        player: &'a Arc<Player>,
        item_stack: &'a mut ItemStack,
    ) -> EntityBaseFuture<'a, bool> {
        Box::pin(async move {
            if item_stack.item.id == Item::IRON_INGOT.id {
                let current_health = self.mob_entity.living_entity.health.load();
                let max_health = self.mob_entity.living_entity.get_max_health();
                if current_health < max_health {
                    self.mob_entity.living_entity.heal(25.0);
                    item_stack.decrement_unless_creative(player.gamemode.load(), 1);
                    let entity = &self.mob_entity.living_entity.entity;
                    let pos = entity.pos.load();
                    let pitch = 1.0 + (rand::random::<f32>() - rand::random::<f32>()) * 0.2;
                    entity.world.load().play_sound_fine(
                        Sound::EntityIronGolemRepair,
                        SoundCategory::Neutral,
                        &pos,
                        1.0,
                        pitch,
                    );
                    return true;
                }
            }
            self.mob_entity.mob_interact(player, item_stack).await
        })
    }

    fn after_attack<'a>(
        &'a self,
        target: &'a dyn EntityBase,
        successful: bool,
    ) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move {
            let entity = &self.mob_entity.living_entity.entity;
            let world = entity.world.load();
            let pos = entity.pos.load();
            world.send_entity_status(entity, EntityStatus::StartAttacking, None);
            world.play_sound(Sound::EntityIronGolemAttack, SoundCategory::Neutral, &pos);
            if successful {
                let resistance = target
                    .cast_any()
                    .downcast_ref::<LivingEntity>()
                    .map_or(0.0, |l| {
                        l.get_attribute_value(&pumpkin_data::attributes::Attributes::KNOCKBACK_RESISTANCE)
                    });
                let scale = (1.0 - resistance).max(0.0);
                if scale > 0.0 {
                    let mut vel = target.get_entity().velocity.load();
                    vel.y += 0.4000000059604645 * scale;
                    target.get_entity().velocity.store(vel);
                    target.get_entity().send_velocity();
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crackiness_levels() {
        assert_eq!(Crackiness::from_health(100.0, 100.0), Crackiness::None);
        assert_eq!(Crackiness::from_health(75.0, 100.0), Crackiness::None);
        assert_eq!(Crackiness::from_health(74.9, 100.0), Crackiness::Low);
        assert_eq!(Crackiness::from_health(50.0, 100.0), Crackiness::Low);
        assert_eq!(Crackiness::from_health(49.9, 100.0), Crackiness::Medium);
        assert_eq!(Crackiness::from_health(25.0, 100.0), Crackiness::Medium);
        assert_eq!(Crackiness::from_health(24.9, 100.0), Crackiness::High);
        assert_eq!(Crackiness::from_health(0.0, 100.0), Crackiness::High);
    }

    #[test]
    fn iron_golem_tracked_data_flags() {
        assert_eq!(TrackedData::IRON_GOLEM_FLAGS.v1_21, 16);
    }

    #[test]
    fn iron_golem_player_created_nbt() {
        let mut compound = pumpkin_nbt::compound::NbtCompound::new();
        compound.put_bool("PlayerCreated", true);
        assert_eq!(compound.get_bool("PlayerCreated"), Some(true));
    }
}
