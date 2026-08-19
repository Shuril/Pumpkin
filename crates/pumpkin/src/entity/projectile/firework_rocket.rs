use crate::{
    entity::{
        Entity, EntityBase, EntityBaseFuture, NBTStorage,
        projectile::{ProjectileHit, ThrownItemEntity},
    },
    server::Server,
    world::World,
};
use pumpkin_data::{
    damage::DamageType, data_component_impl::FireworksImpl, entity::EntityStatus, item::Item,
    item_stack::ItemStack, meta_data_type::MetaDataType, tracked_data::TrackedData,
};
use pumpkin_protocol::bedrock::server::actor_event::ActorEventType;
use pumpkin_protocol::{
    codec::{item_stack_seralizer::ItemStackSerializer, optional_int::OptionalInt},
    java::client::play::Metadata,
};
use pumpkin_util::{
    math::{boundingbox::BoundingBox, vector3::Vector3},
    random::{RandomGenerator, RandomImpl, get_seed, xoroshiro128::Xoroshiro},
};
use std::sync::atomic::AtomicBool;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
use tokio::sync::RwLock;

const GRAVITY: f64 = 0.0;

#[inline]
fn firework_lifetime(flight_duration: i32, random_a: u32, random_b: u32) -> u32 {
    10 * (flight_duration.clamp(0, 127) as u32 + 1) + random_a + random_b
}

#[inline]
fn firework_flight_duration(item_stack: &ItemStack) -> i32 {
    item_stack
        .get_data_component::<FireworksImpl>()
        .map_or(1, |fireworks| fireworks.flight_duration.clamp(0, 127))
}

#[inline]
fn firework_damage(explosion_count: usize, distance: f64) -> f32 {
    if explosion_count == 0 || !(0.0..=5.0).contains(&distance) {
        return 0.0;
    }
    (5.0 + explosion_count as f64 * 2.0) as f32 * ((5.0 - distance) / 5.0).max(0.0).sqrt() as f32
}

pub struct FireworkRocketEntity {
    entity: ThrownItemEntity,
    item_stack: RwLock<ItemStack>,
    life: AtomicU32,
    life_time: AtomicU32,
}

impl FireworkRocketEntity {
    pub fn new(entity: Entity) -> Self {
        Self::new_with_item(entity, ItemStack::new(1, &Item::FIREWORK_ROCKET))
    }

    pub fn new_with_item(entity: Entity, item_stack: ItemStack) -> Self {
        let mut random = RandomGenerator::Xoroshiro(Xoroshiro::from_seed(get_seed()));

        entity.set_velocity(Vector3::new(
            random.next_triangular(0.0, 0.002_297),
            0.05,
            random.next_triangular(0.0, 0.002_297),
        ));
        let flight_duration = firework_flight_duration(&item_stack);
        Self {
            entity: ThrownItemEntity {
                entity,
                owner_id: None,
                collides_with_projectiles: false,
                has_hit: AtomicBool::new(false),
                gravity: GRAVITY,
            },
            item_stack: RwLock::new(item_stack),
            life: 0.into(),
            life_time: firework_lifetime(
                flight_duration,
                random.next_bounded_i32(6) as u32,
                random.next_bounded_i32(7) as u32,
            )
            .into(),
        }
    }

    pub fn new_shot(entity: Entity, shooter: &Entity) -> Self {
        Self::new_shot_with_item(entity, shooter, ItemStack::new(1, &Item::FIREWORK_ROCKET))
    }

    pub fn new_shot_with_item(entity: Entity, shooter: &Entity, item_stack: ItemStack) -> Self {
        let mut random = RandomGenerator::Xoroshiro(Xoroshiro::from_seed(get_seed()));

        // Set random initial velocity
        // Set on the inner entity after constructing ThrownItemEntity
        let thrown = ThrownItemEntity::new(entity, shooter, GRAVITY);
        thrown.entity.set_velocity(Vector3::new(
            random.next_triangular(0.0, 0.002_297),
            0.05,
            random.next_triangular(0.0, 0.002_297),
        ));

        // Set random life
        let flight_duration = firework_flight_duration(&item_stack);
        let rocket = Self {
            entity: thrown,
            item_stack: RwLock::new(item_stack),
            life: 0.into(),
            life_time: firework_lifetime(
                flight_duration,
                random.next_bounded_i32(6) as u32,
                random.next_bounded_i32(7) as u32,
            )
            .into(),
        };

        // Set shooter metadata
        rocket.entity.entity.send_meta_data(
            &[Metadata::new(
                TrackedData::ATTACHED_TO_TARGET,
                MetaDataType::OPTIONAL_INT,
                OptionalInt(Some(shooter.entity_id)),
            )],
            None,
        );

        rocket
    }

    pub async fn explode_and_remove(&self, world: &World) {
        let entity = self.get_entity();
        let explosion_count = self
            .item_stack
            .read()
            .await
            .get_data_component::<FireworksImpl>()
            .map_or(0, |fireworks| fireworks.explosions.len());

        // Vanilla only damages living entities when at least one explosion
        // shape is present.  The radius is five blocks and each target must be
        // visible from the rocket (the client still receives the status event
        // for empty/no-effect rockets).
        if explosion_count > 0 {
            let center = entity.pos.load();
            let search_box = BoundingBox::new(
                center - Vector3::new(5.0, 5.0, 5.0),
                center + Vector3::new(5.0, 5.0, 5.0),
            );
            for target in world.get_all_at_box(&search_box) {
                if target.get_entity().entity_id == entity.entity_id
                    || target.get_living_entity().is_none()
                {
                    continue;
                }
                let distance = target
                    .get_entity()
                    .pos
                    .load()
                    .squared_distance_to_vec(&center)
                    .sqrt();
                let damage = firework_damage(explosion_count, distance);
                if damage <= 0.0 {
                    continue;
                }
                let target_pos = target.get_eye_pos();
                let blocked = entity
                    .world
                    .load()
                    .raycast(center, target_pos, async |candidate, world_ref| {
                        let state = world_ref.get_block_state(candidate);
                        !state.is_air() && !state.collision_shapes.is_empty()
                    })
                    .await
                    .is_some();
                if !blocked {
                    target
                        .damage(target.as_ref(), damage, DamageType::FIREWORKS)
                        .await;
                }
            }
        }
        world.send_entity_status(
            entity,
            EntityStatus::FireworksExplode,
            Some(ActorEventType::FireworksExplode),
        );

        entity.remove().await;
    }
}

#[cfg(test)]
mod tests {
    use super::{firework_damage, firework_flight_duration, firework_lifetime};
    use pumpkin_data::data_component::DataComponent;
    use pumpkin_data::data_component_impl::FireworksImpl;
    use pumpkin_data::item::Item;
    use pumpkin_data::item_stack::ItemStack;

    #[test]
    fn lifetime_uses_vanilla_flight_duration_and_random_bounds() {
        assert_eq!(firework_lifetime(0, 0, 0), 10);
        assert_eq!(firework_lifetime(0, 5, 6), 21);
        assert_eq!(firework_lifetime(3, 2, 4), 46);
        assert_eq!(firework_lifetime(-5, 0, 0), 10);
    }

    #[test]
    fn item_component_controls_flight_duration() {
        let default = ItemStack::new(1, &Item::FIREWORK_ROCKET);
        assert_eq!(firework_flight_duration(&default), 1);

        let mut component = FireworksImpl::new(3, Vec::new());
        let configured = ItemStack::new_with_component(
            1,
            &Item::FIREWORK_ROCKET,
            vec![(DataComponent::Fireworks, Some(Box::new(component.clone())))],
        );
        assert_eq!(firework_flight_duration(&configured), 3);

        component.flight_duration = 255;
        let configured = ItemStack::new_with_component(
            1,
            &Item::FIREWORK_ROCKET,
            vec![(DataComponent::Fireworks, Some(Box::new(component)))],
        );
        assert_eq!(firework_flight_duration(&configured), 127);
    }

    #[test]
    fn explosion_damage_matches_vanilla_radius_and_scaling() {
        assert_eq!(firework_damage(0, 0.0), 0.0);
        assert_eq!(firework_damage(1, 5.1), 0.0);
        assert!((firework_damage(1, 0.0) - 7.0).abs() < f32::EPSILON);
        assert!((firework_damage(2, 2.5) - 6.363_961).abs() < 0.000_01);
    }
}

impl NBTStorage for FireworkRocketEntity {
    fn write_nbt<'a>(
        &'a self,
        nbt: &'a mut pumpkin_nbt::compound::NbtCompound,
    ) -> crate::entity::NbtFuture<'a, ()> {
        Box::pin(async move {
            self.entity.entity.write_nbt(nbt).await;
            nbt.put_int("Life", self.life.load(Ordering::Relaxed) as i32);
            nbt.put_int("LifeTime", self.life_time.load(Ordering::Relaxed) as i32);
            let mut item = pumpkin_nbt::compound::NbtCompound::new();
            self.item_stack.read().await.write_item_stack(&mut item);
            nbt.put_compound("FireworksItem", item);
        })
    }

    fn read_nbt_non_mut<'a>(
        &'a self,
        nbt: &'a pumpkin_nbt::compound::NbtCompound,
    ) -> crate::entity::NbtFuture<'a, ()> {
        Box::pin(async move {
            self.entity.entity.read_nbt_non_mut(nbt).await;
            self.life.store(
                nbt.get_int("Life").unwrap_or(0).max(0) as u32,
                Ordering::Relaxed,
            );
            if let Some(lifetime) = nbt.get_int("LifeTime") {
                self.life_time
                    .store(lifetime.max(0) as u32, Ordering::Relaxed);
            }
            if let Some(item) = nbt
                .get_compound("FireworksItem")
                .and_then(ItemStack::read_item_stack)
            {
                *self.item_stack.write().await = item;
            }
        })
    }
}

impl EntityBase for FireworkRocketEntity {
    fn init_data_tracker(&self) -> EntityBaseFuture<'_, ()> {
        Box::pin(async move {
            let stack = self.item_stack.read().await;
            self.get_entity().send_meta_data(
                &[Metadata::new(
                    TrackedData::ID_FIREWORKS_ITEM,
                    MetaDataType::ITEM_STACK,
                    &ItemStackSerializer::from(stack.clone()),
                )],
                None,
            );
        })
    }

    fn tick<'a>(
        &'a self,
        caller: &'a Arc<dyn EntityBase>,
        server: &'a Server,
    ) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move {
            self.entity.process_tick(caller, server).await;

            let entity = self.get_entity();
            let world = entity.world.load();
            let mut velocity = entity.velocity.load();

            if let Some(shooter_id) = self.entity.owner_id {
                // Check if the player who fired this rocket still exists in the world
                if let Some(shooter) = world.get_entity_by_id(shooter_id) {
                    let shooter = shooter.get_entity();

                    // Logic for boosting Elytra flight
                    if shooter.is_fall_flying() {
                        let rotation = shooter.rotation().to_f64();
                        let shooter_vel = shooter.velocity.load();

                        let new_shooter_vel =
                            shooter_vel + (rotation * 0.1 + (rotation * 1.5 - shooter_vel) * 0.5);

                        shooter.set_velocity(new_shooter_vel);

                        entity.set_pos(shooter.pos.load());
                        entity.set_velocity(new_shooter_vel);
                    }
                }
            } else {
                // Standard firework rocket flight logic
                velocity.x *= 1.15;
                velocity.z *= 1.15;
                velocity.y += 0.04;
                entity.set_velocity(velocity);
            }

            // Increment life and check for explosion
            let current_life = self.life.fetch_add(1, Ordering::Relaxed);
            if current_life > self.life_time.load(Ordering::Relaxed) {
                self.explode_and_remove(&world).await;
            }
        })
    }

    fn on_hit(&self, hit: ProjectileHit) -> EntityBaseFuture<'_, ()> {
        Box::pin(async move {
            let should_explode = match hit {
                // Vanilla detonates on entity impact regardless of whether the
                // item carries explosion components.  A block impact only
                // detonates a rocket that has at least one explosion shape.
                ProjectileHit::Entity { .. } => true,
                ProjectileHit::Block { .. } => self
                    .item_stack
                    .read()
                    .await
                    .get_data_component::<FireworksImpl>()
                    .is_some_and(|fireworks| !fireworks.explosions.is_empty()),
            };
            if should_explode {
                let world = self.get_entity().world.load();
                self.explode_and_remove(&world).await;
            }
        })
    }

    fn get_entity(&self) -> &crate::entity::Entity {
        &self.entity.entity
    }

    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }

    fn as_nbt_storage(&self) -> &dyn crate::entity::NBTStorage {
        self
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn projectile_owner_id(&self) -> Option<i32> {
        self.entity.owner_id
    }
}
