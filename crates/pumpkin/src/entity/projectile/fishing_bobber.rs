use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use crate::entity::projectile::{ProjectileHit, is_projectile};
use crate::{
    entity::{
        Entity, EntityBase, EntityBaseFuture, NBTStorage, living::LivingEntity, player::Player,
    },
    server::Server,
    world::World,
};
use pumpkin_data::meta_data_type::MetaDataType;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tracked_data::TrackedData;
use pumpkin_data::{Block, fluid::Fluid};
use pumpkin_protocol::java::client::play::Metadata;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos};

pub struct FishingBobberEntity {
    pub entity: Entity,
    pub owner_id: i32,
    pub hooked_entity_id: AtomicI32,
    pub in_ground: AtomicBool,
    pub has_hit: AtomicBool,
    pub wait_countdown: AtomicI32,
    pub bite_countdown: AtomicI32,
    pub open_water: AtomicBool,
}

/// Vanilla's `FishingHook#retrieve` damage table.
///
/// A hooked item entity costs three durability, every other hooked entity costs
/// five, a successful loot roll costs one, and a bobber that is stuck in a
/// block costs two.  The on-ground cost is applied last by vanilla and thus
/// overrides the other outcomes.
#[must_use]
pub const fn retrieval_damage(
    hooked: bool,
    hooked_item_entity: bool,
    has_catch: bool,
    on_ground: bool,
) -> i32 {
    let mut damage = if hooked {
        if hooked_item_entity { 3 } else { 5 }
    } else if has_catch {
        1
    } else {
        0
    };
    if on_ground {
        damage = 2;
    }
    damage
}

impl FishingBobberEntity {
    const WATER_INERTIA: f64 = 0.8;
    const AIR_INERTIA: f64 = 0.92;
    const GRAVITY: f64 = 0.03;

    pub fn new(entity: Entity, owner: &Player) -> Self {
        let mut owner_pos = owner.living_entity.entity.pos.load();
        owner_pos.y += owner.living_entity.entity.get_eye_height() - 0.1;
        entity.set_pos(owner_pos);

        Self {
            entity,
            owner_id: owner.living_entity.entity.entity_id,
            hooked_entity_id: AtomicI32::new(0),
            in_ground: AtomicBool::new(false),
            has_hit: AtomicBool::new(false),
            wait_countdown: AtomicI32::new(rand::random::<i32>().abs() % 600 + 100),
            bite_countdown: AtomicI32::new(0),
            open_water: AtomicBool::new(false),
        }
    }

    /// Mirrors `FishingHook#calculateOpenWater`: every 5×5 layer from one
    /// block below through two blocks above must be uniformly above-water or
    /// source-water, with no solid collision blocks or mixed layers.
    fn calculate_open_water(world: &World, center: BlockPos) -> bool {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Layer {
            AboveWater,
            InsideWater,
        }

        let mut previous = None;
        for y in -1..=2 {
            let mut layer = None;
            for x in -2..=2 {
                for z in -2..=2 {
                    let position = BlockPos::new(center.0.x + x, center.0.y + y, center.0.z + z);
                    let state_id = world.get_block_state_id(&position);
                    let state = world.get_block_state(&position);
                    let block = Block::from_state_id(state_id);
                    let current = if state.is_air() || block.id == Block::LILY_PAD.id {
                        Layer::AboveWater
                    } else if !state.is_solid()
                        && Fluid::from_state_id(state_id).is_some_and(|fluid| {
                            fluid.matches_type(&Fluid::WATER) && fluid.is_source(state_id)
                        })
                    {
                        Layer::InsideWater
                    } else {
                        return false;
                    };
                    if layer.is_some_and(|existing| existing != current) {
                        return false;
                    }
                    layer = Some(current);
                }
            }
            let Some(layer) = layer else {
                return false;
            };
            if (previous.is_none() && layer == Layer::AboveWater)
                || (previous == Some(Layer::AboveWater) && layer == Layer::InsideWater)
            {
                return false;
            }
            previous = Some(layer);
        }
        true
    }

    pub async fn reel_in(&self, player: &Player) -> i32 {
        use pumpkin_data::item::Item;
        let world = self.entity.world.load();
        let hooked_id = self.hooked_entity_id.load(Ordering::Relaxed);

        if hooked_id != 0
            && let Some(hooked) = world.get_entity_by_id(hooked_id)
        {
            let player_pos = player.get_entity().pos.load();
            let hooked_pos = hooked.get_entity().pos.load();
            let delta = player_pos - hooked_pos;
            let motion =
                delta
                    .multiply(0.1, 0.1, 0.1)
                    .add_raw(0.0, delta.length().sqrt() * 0.08, 0.0);
            hooked.get_entity().add_velocity(motion);
            return retrieval_damage(
                true,
                hooked.get_item_entity().is_some(),
                false,
                self.entity.on_ground.load(Ordering::Relaxed),
            );
        }

        if self.bite_countdown.load(Ordering::Relaxed) > 0 {
            // Caught something!
            let world_time = world.level_info.load().day_time as u64;
            let seed = crate::world::loot::derive_loot_seed(
                world.level.seed.0,
                Some(self.entity.pos.load()),
                world_time,
                self.entity.entity_id as u64 ^ 0x4649_5348,
            ) as i64;
            let resources = world
                .server
                .upgrade()
                .map(|server| async move { server.recipe_manager.datapack_resources().await });
            let resources = match resources {
                Some(future) => Some(future.await),
                None => None,
            };
            let dynamic = resources.as_ref().and_then(|resources| {
                resources
                    .loot_tables
                    .contains_key("minecraft:gameplay/fishing")
                    .then(|| {
                        crate::world::loot::generate_datapack_chest_loot(
                            resources,
                            "minecraft:gameplay/fishing",
                            seed,
                        )
                    })
            });
            let items = match dynamic {
                Some(Ok(items)) => items,
                Some(Err(error)) => {
                    tracing::warn!("Could not evaluate datapack fishing loot table: {error}");
                    crate::world::loot::generate_builtin_fishing_loot(
                        seed,
                        self.open_water.load(Ordering::Relaxed),
                    )
                }
                None => crate::world::loot::generate_builtin_fishing_loot(
                    seed,
                    self.open_water.load(Ordering::Relaxed),
                ),
            };

            for item_stack in items {
                if [
                    Item::COD.id,
                    Item::SALMON.id,
                    Item::TROPICAL_FISH.id,
                    Item::PUFFERFISH.id,
                ]
                .contains(&item_stack.item.id)
                {
                    player
                        .increment_stat(
                            pumpkin_data::statistic::StatisticCategory::Custom,
                            pumpkin_data::statistic::CustomStatistic::FishCaught as i32,
                            1,
                        )
                        .await;
                }
                player
                    .inventory
                    .offer_or_drop_stack(item_stack.clone(), player)
                    .await;
                player
                    .trigger_advancement(
                        crate::entity::player::advancement::trigger::AdvancementTrigger::FishedItem {
                            item_id: format!("minecraft:{}", item_stack.item.registry_key),
                        },
                    )
                    .await;
            }

            world.play_sound(
                Sound::EntityExperienceOrbPickup,
                SoundCategory::Neutral,
                &player.position(),
            );
            return retrieval_damage(
                false,
                false,
                true,
                self.entity.on_ground.load(Ordering::Relaxed),
            );
        }

        retrieval_damage(
            false,
            false,
            false,
            self.entity.on_ground.load(Ordering::Relaxed),
        )
    }

    #[expect(clippy::too_many_lines)]
    pub async fn process_tick<'a>(&'a self, caller: &'a Arc<dyn EntityBase>, _server: &'a Server) {
        let entity = self.get_entity();
        let world = entity.world.load();

        if self.in_ground.load(Ordering::Relaxed) {
            return;
        }

        let hooked_id = self.hooked_entity_id.load(Ordering::Relaxed);
        if hooked_id != 0 {
            if let Some(hooked) = world.get_entity_by_id(hooked_id) {
                if hooked.get_entity().removed.load(Ordering::Relaxed) {
                    self.hooked_entity_id.store(0, Ordering::Relaxed);
                } else {
                    let mut hooked_pos = hooked.get_entity().pos.load();
                    hooked_pos.y += hooked.get_entity().get_eye_height() * 0.8;
                    entity.set_pos(hooked_pos);
                    return;
                }
            } else {
                self.hooked_entity_id.store(0, Ordering::Relaxed);
            }
        }

        let mut velocity = entity.velocity.load();
        let start_pos = entity.pos.load();

        if entity.touching_water.load(Ordering::Relaxed) {
            self.open_water.store(
                Self::calculate_open_water(&world, entity.block_pos.load()),
                Ordering::Relaxed,
            );
            velocity.y += 0.02; // Buoyancy

            let bite = self.bite_countdown.load(Ordering::Relaxed);
            if bite > 0 {
                self.bite_countdown.store(bite - 1, Ordering::Relaxed);
                if bite % 5 == 0 {
                    world.spawn_particle(
                        entity.pos.load(),
                        Vector3::new(0.1f32, 0.1f32, 0.1f32),
                        0.0,
                        5,
                        pumpkin_data::particle::Particle::Bubble,
                    );
                }
            } else {
                let wait = self.wait_countdown.load(Ordering::Relaxed);
                if wait > 0 {
                    self.wait_countdown.store(wait - 1, Ordering::Relaxed);
                } else {
                    // Start bite
                    self.bite_countdown.store(40, Ordering::Relaxed);
                    self.wait_countdown
                        .store(rand::random::<i32>().abs() % 600 + 100, Ordering::Relaxed);

                    world.play_sound(
                        Sound::EntityFishingBobberSplash,
                        SoundCategory::Neutral,
                        &entity.pos.load(),
                    );
                }
            }
        } else {
            velocity.y -= Self::GRAVITY;
        }

        let inertia = if entity.touching_water.load(Ordering::Relaxed) {
            Self::WATER_INERTIA
        } else {
            Self::AIR_INERTIA
        };
        velocity = velocity.multiply(inertia, inertia, inertia);
        entity.velocity.store(velocity);

        let new_pos = start_pos.add(&velocity);

        let search_box = BoundingBox::new(
            Vector3::new(
                start_pos.x.min(new_pos.x),
                start_pos.y.min(new_pos.y),
                start_pos.z.min(new_pos.z),
            ),
            Vector3::new(
                start_pos.x.max(new_pos.x),
                start_pos.y.max(new_pos.y),
                start_pos.z.max(new_pos.z),
            ),
        )
        .expand(0.3, 0.3, 0.3);

        // Basic block collision to stop bobber
        let (block_cols, _) = world
            .get_block_collisions(search_box, caller.as_ref())
            .await;
        if !block_cols.is_empty() {
            self.in_ground.store(true, Ordering::Relaxed);
            entity.velocity.store(Vector3::new(0.0, 0.0, 0.0));
            return;
        }

        entity.set_pos(new_pos);

        let candidates = world.get_entities_at_box(&search_box);
        for cand in candidates {
            if cand.get_entity().entity_id == self.owner_id
                || cand.get_entity().entity_id == entity.entity_id
            {
                continue;
            }

            if is_projectile(cand.get_entity().entity_type) {
                continue;
            }

            let ebb = cand.get_entity().bounding_box.load().expand(0.3, 0.3, 0.3);
            if ebb.intersects(&search_box) {
                self.hooked_entity_id
                    .store(cand.get_entity().entity_id, Ordering::Relaxed);
                entity.send_meta_data(
                    &[Metadata::new(
                        TrackedData::HOOKED_ENTITY,
                        MetaDataType::INT,
                        cand.get_entity().entity_id + 1,
                    )],
                    None,
                );
                return;
            }
        }
    }
}

impl NBTStorage for FishingBobberEntity {}

impl EntityBase for FishingBobberEntity {
    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_nbt_storage(&self) -> &dyn NBTStorage {
        self
    }

    fn on_hit(&self, _hit: ProjectileHit) -> EntityBaseFuture<'_, ()> {
        Box::pin(async move {
            self.has_hit.store(true, Ordering::Relaxed);
        })
    }

    fn tick<'a>(
        &'a self,
        caller: &'a Arc<dyn EntityBase>,
        server: &'a Server,
    ) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move {
            self.process_tick(caller, server).await;
        })
    }
}

#[cfg(test)]
mod tests {
    use super::retrieval_damage;

    #[test]
    fn fishing_retrieval_damage_matches_vanilla_outcomes() {
        assert_eq!(retrieval_damage(true, true, false, false), 3);
        assert_eq!(retrieval_damage(true, false, false, false), 5);
        assert_eq!(retrieval_damage(false, false, true, false), 1);
        assert_eq!(retrieval_damage(false, false, false, true), 2);
        assert_eq!(retrieval_damage(true, false, true, true), 2);
        assert_eq!(retrieval_damage(false, false, false, false), 0);
    }
}
