use rand::RngExt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::entity::experience_orb::ExperienceOrbEntity;
use crate::entity::projectile::ThrownItemEntity;
use crate::{
    entity::{Entity, EntityBase, EntityBaseFuture, NBTStorage},
    server::Server,
};
use pumpkin_data::entity::EntityStatus;
use pumpkin_protocol::bedrock::server::actor_event::ActorEventType;
use pumpkin_util::math::vector3::Vector3;

const GRAVITY: f64 = 0.07;

/// Server-side equivalent of Java's `ThrownExperienceBottle`.
/// The bottle is consumed at launch and creates 3..=11 experience only when
/// its projectile collision is processed.
pub struct ExperienceBottleEntity {
    pub thrown: ThrownItemEntity,
}

impl ExperienceBottleEntity {
    pub fn new(entity: Entity) -> Self {
        entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));
        Self {
            thrown: ThrownItemEntity {
                entity,
                owner_id: None,
                collides_with_projectiles: false,
                has_hit: AtomicBool::new(false),
                gravity: GRAVITY,
            },
        }
    }
}

impl NBTStorage for ExperienceBottleEntity {}

impl EntityBase for ExperienceBottleEntity {
    fn tick<'a>(
        &'a self,
        caller: &'a Arc<dyn EntityBase>,
        server: &'a Server,
    ) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move { self.thrown.process_tick(caller, server).await })
    }

    fn get_entity(&self) -> &Entity {
        self.thrown.get_entity()
    }

    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }

    fn as_nbt_storage(&self) -> &dyn NBTStorage {
        self
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn projectile_owner_id(&self) -> Option<i32> {
        self.thrown.owner_id
    }

    fn on_hit(&self, hit: crate::entity::projectile::ProjectileHit) -> EntityBaseFuture<'_, ()> {
        Box::pin(async move {
            let world = self.get_entity().world.load();
            let hit_pos = hit.hit_pos();
            world.send_entity_status(
                self.get_entity(),
                EntityStatus::Death,
                Some(ActorEventType::Death),
            );
            let amount = {
                let mut random = rand::rng();
                random.random_range(3..=11)
            };
            ExperienceOrbEntity::spawn(&world, hit_pos, amount).await;
        })
    }
}
