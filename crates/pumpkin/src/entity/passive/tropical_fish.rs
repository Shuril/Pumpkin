use std::sync::{
    Arc, Weak,
    atomic::{AtomicI32, Ordering},
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

pub struct TropicalFishEntity {
    pub mob_entity: MobEntity,
    pub variant: AtomicI32,
}

impl TropicalFishEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let tropical_fish = Self {
            mob_entity,
            variant: AtomicI32::new(0),
        };
        let mob_arc = Arc::new(tropical_fish);
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

impl TropicalFishEntity {
    pub fn set_variant_components(&self, pattern: &str, base_color: &str, pattern_color: &str) {
        let pattern_id = tropical_pattern_id(pattern);
        let base_id = dye_color_id(base_color);
        let pattern_color_id = dye_color_id(pattern_color);
        self.variant.store(
            (pattern_id | (base_id << 16) | (pattern_color_id << 24)) as i32,
            Ordering::Relaxed,
        );
    }
}

impl NBTStorage for TropicalFishEntity {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.write_nbt(nbt).await;
            nbt.put_int("Variant", self.variant.load(Ordering::Relaxed));
        })
    }

    fn read_nbt_non_mut<'a>(&'a self, nbt: &'a NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.read_nbt_non_mut(nbt).await;
            if let Some(variant) = nbt.get_int("Variant") {
                self.variant.store(variant, Ordering::Relaxed);
            }
        })
    }
}

impl Mob for TropicalFishEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_init_data_tracker(&self) -> EntityBaseFuture<'_, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.entity.send_meta_data(
                &[Metadata::new(
                    TrackedData::ID_TYPE_VARIANT,
                    MetaDataType::INT,
                    VarInt(self.variant.load(Ordering::Relaxed)),
                )],
                None,
            );
        })
    }
}

fn tropical_pattern_id(value: &str) -> i32 {
    match value.strip_prefix("minecraft:").unwrap_or(value) {
        "sunstreak" => 1 | (0 << 8),
        "snooper" => 2 | (0 << 8),
        "dasher" => 3 | (0 << 8),
        "brinely" => 4 | (0 << 8),
        "spotty" => 5 | (0 << 8),
        "flopper" => 0 | (1 << 8),
        "stripey" => 1 | (1 << 8),
        "glitter" => 2 | (1 << 8),
        "blockfish" => 3 | (1 << 8),
        "betty" => 4 | (1 << 8),
        "clayfish" => 5 | (1 << 8),
        _ => 0,
    }
}

fn dye_color_id(value: &str) -> i32 {
    match value.strip_prefix("minecraft:").unwrap_or(value) {
        "orange" => 1,
        "magenta" => 2,
        "light_blue" => 3,
        "yellow" => 4,
        "lime" => 5,
        "pink" => 6,
        "gray" => 7,
        "light_gray" => 8,
        "cyan" => 9,
        "purple" => 10,
        "blue" => 11,
        "brown" => 12,
        "green" => 13,
        "red" => 14,
        "black" => 15,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{dye_color_id, tropical_pattern_id};

    #[test]
    fn tropical_fish_variant_uses_vanilla_packed_layout() {
        let variant = tropical_pattern_id("stripey")
            | (dye_color_id("orange") << 16)
            | (dye_color_id("gray") << 24);
        assert_eq!(variant, 0x0701_0101);
    }

    #[test]
    fn tropical_fish_defaults_to_kob_white_white() {
        assert_eq!(tropical_pattern_id("unknown"), 0);
        assert_eq!(dye_color_id("unknown"), 0);
    }
}
