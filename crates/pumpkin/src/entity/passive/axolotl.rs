use std::sync::{
    Arc, Weak,
    atomic::{AtomicU8, Ordering},
};

use pumpkin_data::entity::EntityType;
use pumpkin_data::meta_data_type::MetaDataType;
use pumpkin_data::tracked_data::TrackedData;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_protocol::java::client::play::Metadata;

use crate::entity::{
    Entity, EntityBaseFuture, NBTStorage, NbtFuture,
    ai::goal::{
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal, swim::SwimGoal,
        wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
};

/// Represents an Axolotl, a passive aquatic mob that can play dead to regenerate health.
///
/// Wiki: <https://minecraft.wiki/w/Axolotl>
pub struct AxolotlEntity {
    pub mob_entity: MobEntity,
    pub variant: AtomicU8,
}

impl AxolotlEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let axolotl = Self {
            mob_entity,
            variant: AtomicU8::new(0),
        };
        let mob_arc = Arc::new(axolotl);
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

            goal_selector.add_goal(0, Box::new(SwimGoal::default()));
            goal_selector.add_goal(1, Box::new(WanderAroundGoal::new(1.0)));
            goal_selector.add_goal(
                2,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(3, Box::new(RandomLookAroundGoal::default()));
        };

        mob_arc
    }
}

impl AxolotlEntity {
    fn set_variant_value(&self, value: &str) {
        let variant = match value.strip_prefix("minecraft:").unwrap_or(value) {
            "wild" => 1,
            "gold" => 2,
            "cyan" => 3,
            "blue" => 4,
            _ => 0,
        };
        self.variant.store(variant, Ordering::Relaxed);
    }
}

impl NBTStorage for AxolotlEntity {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.write_nbt(nbt).await;
            let value = match self.variant.load(Ordering::Relaxed) {
                1 => "wild",
                2 => "gold",
                3 => "cyan",
                4 => "blue",
                _ => "lucy",
            };
            nbt.put_string("Variant", value.to_string());
        })
    }

    fn read_nbt_non_mut<'a>(&'a self, nbt: &'a NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.read_nbt_non_mut(nbt).await;
            if let Some(value) = nbt.get_string("Variant") {
                self.set_variant_value(value);
            }
        })
    }
}

impl Mob for AxolotlEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_set_variant_name(&self, name: &str) {
        self.set_variant_value(name);
    }

    fn mob_init_data_tracker(&self) -> EntityBaseFuture<'_, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.entity.send_meta_data(
                &[Metadata::new(
                    TrackedData::VARIANT_ID,
                    MetaDataType::INT,
                    VarInt(self.variant.load(Ordering::Relaxed).into()),
                )],
                None,
            );
        })
    }
}
