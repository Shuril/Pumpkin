use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::potion::Effect;
use pumpkin_util::math::vector3::Vector3;

use crate::{
    entity::{
        Entity, EntityBase, EntityBaseFuture,
        projectile::{ProjectileHit, ThrownItemEntity},
    },
    server::Server,
};

const GRAVITY: f64 = 0.0;
const IMPACT_DAMAGE: f32 = 8.0;
const WITHER_EFFECT_DURATION: i32 = 800;

pub struct WitherSkullEntity {
    pub thrown: ThrownItemEntity,
}

impl WitherSkullEntity {
    #[must_use]
    pub const fn new(entity: Entity) -> Self {
        let thrown = ThrownItemEntity {
            entity,
            owner_id: None,
            collides_with_projectiles: false,
            has_hit: AtomicBool::new(false),
            gravity: GRAVITY,
        };

        Self { thrown }
    }

    #[must_use]
    pub fn new_shot(entity: Entity, shooter: &Entity) -> Self {
        let thrown = ThrownItemEntity::new(entity, shooter, GRAVITY);
        Self { thrown }
    }

    async fn blast(&self, pos: Vector3<f64>) {
        let world = self.get_entity().world.load();
        let explosion = crate::world::explosion::Explosion::new(
            1.0,
            pos,
            crate::world::explosion::BlockInteraction::Destroy,
        );
        explosion.explode(&world).await;
    }

    async fn wither_target(&self, target: Arc<dyn EntityBase>) {
        if let Some(living) = target.get_living_entity() {
            living
                .add_effect(Effect {
                    effect_type: &StatusEffect::WITHER,
                    duration: WITHER_EFFECT_DURATION,
                    amplifier: 0,
                    ambient: false,
                    show_particles: true,
                    show_icon: true,
                    blend: false,
                })
                .await;
        }
        let _ = target
            .damage(target.as_ref(), IMPACT_DAMAGE, DamageType::WITHER_SKULL)
            .await;
    }
}

impl EntityBase for WitherSkullEntity {
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

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_hit(&self, hit: ProjectileHit) -> EntityBaseFuture<'_, ()> {
        Box::pin(async move {
            let pos = self.get_entity().pos.load();
            match hit {
                ProjectileHit::Entity { ref entity, .. } => {
                    self.blast(pos).await;
                    self.wither_target(entity.clone()).await;
                }
                ProjectileHit::Block { .. } => {
                    self.blast(pos).await;
                }
            }
        })
    }
}
