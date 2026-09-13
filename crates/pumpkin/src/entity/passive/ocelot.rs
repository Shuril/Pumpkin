use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use pumpkin_data::entity::{EntityStatus, EntityType};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::meta_data_type::MetaDataType;
use pumpkin_data::particle::Particle;
use pumpkin_data::tracked_data::TrackedData;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::java::client::play::Metadata;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use crate::entity::{
    Entity, EntityBase, EntityBaseFuture, NBTStorage, NbtFuture,
    ai::goal::{
        active_target::ActiveTargetGoal, avoid_entity::AvoidEntityGoal, breed::BreedGoal,
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal, swim::SwimGoal,
        tempt::TemptGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    player::Player,
};

const TEMPT_ITEMS: &[&Item] = &[&Item::COD, &Item::SALMON];

/// Represents an Ocelot, a shy passive mob found in jungles.
///
/// Wiki: <https://minecraft.wiki/w/Ocelot>
pub struct OcelotEntity {
    pub mob_entity: MobEntity,
    pub is_trusting: AtomicBool,
}

impl OcelotEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let ocelot = Self {
            mob_entity,
            is_trusting: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(ocelot);
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
            goal_selector.add_goal(
                4,
                Box::new(AvoidEntityGoal::new(&EntityType::PLAYER, 16.0, 0.8, 1.33)),
            );
            goal_selector.add_goal(3, Box::new(TemptGoal::new(0.6, TEMPT_ITEMS)));
            goal_selector.add_goal(9, BreedGoal::new(0.8));
            goal_selector.add_goal(10, Box::new(WanderAroundGoal::new(0.8)));
            goal_selector.add_goal(
                11,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 10.0),
            );
            goal_selector.add_goal(11, Box::new(RandomLookAroundGoal::default()));

            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::CHICKEN, false),
            );
            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::TURTLE, false),
            );
        };

        mob_arc
    }

    pub fn is_trusting(&self) -> bool {
        self.is_trusting.load(Ordering::Relaxed)
    }

    pub fn set_trusting(&self, trusting: bool) {
        self.is_trusting.store(trusting, Ordering::Relaxed);
        self.mob_entity.living_entity.entity.send_meta_data(
            &[Metadata::new(
                TrackedData::TRUSTING,
                MetaDataType::BOOLEAN,
                trusting,
            )],
            None,
        );
    }
}

impl NBTStorage for OcelotEntity {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.write_nbt(nbt).await;
            nbt.put_bool("Trusting", self.is_trusting());
        })
    }

    fn read_nbt_non_mut<'a>(&'a self, nbt: &'a NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.read_nbt_non_mut(nbt).await;
            if let Some(trusting) = nbt.get_bool("Trusting") {
                self.is_trusting.store(trusting, Ordering::Relaxed);
            }
        })
    }
}

impl Mob for OcelotEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_init_data_tracker(&self) -> EntityBaseFuture<'_, ()> {
        Box::pin(async move {
            let entity = self.get_entity();
            let is_baby = entity.age.load(Ordering::Relaxed) < 0;
            if is_baby {
                entity.send_meta_data(
                    &[Metadata::new(
                        TrackedData::BABY_ID,
                        MetaDataType::BOOLEAN,
                        true,
                    )],
                    None,
                );
            }
            entity.send_meta_data(
                &[Metadata::new(
                    TrackedData::TRUSTING,
                    MetaDataType::BOOLEAN,
                    self.is_trusting(),
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
            let entity = self.get_entity();
            let world = entity.world.load();
            let is_food =
                item_stack.item.id == Item::COD.id || item_stack.item.id == Item::SALMON.id;

            let dist_sq = entity
                .pos
                .load()
                .squared_distance_to_vec(&player.living_entity.entity.pos.load());
            if !self.is_trusting() && is_food && dist_sq < 9.0 {
                item_stack.decrement_unless_creative(player.gamemode.load(), 1);
                let chance = rand::rng().random_range(0..3);
                if chance == 0 {
                    self.set_trusting(true);
                    world.send_entity_status(entity, EntityStatus::TrustingSucceeded, None);
                    world.spawn_particle(
                        entity.pos.load()
                            + Vector3::new(0.0, f64::from(entity.height()) * 0.5, 0.0),
                        Vector3::new(0.5, 0.5, 0.5),
                        1.0,
                        7,
                        Particle::Heart,
                    );
                } else {
                    world.send_entity_status(entity, EntityStatus::TrustingFailed, None);
                    world.spawn_particle(
                        entity.pos.load()
                            + Vector3::new(0.0, f64::from(entity.height()) * 0.5, 0.0),
                        Vector3::new(0.5, 0.5, 0.5),
                        1.0,
                        7,
                        Particle::Smoke,
                    );
                }
                return true;
            }

            self.mob_entity.mob_interact(player, item_stack).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocelot_tracked_data_trusting() {
        assert_eq!(TrackedData::TRUSTING.v1_21, 17);
    }

    #[test]
    fn ocelot_nbt_roundtrip() {
        let mut compound = NbtCompound::new();
        compound.put_bool("Trusting", true);
        assert_eq!(compound.get_bool("Trusting"), Some(true));
    }
}
