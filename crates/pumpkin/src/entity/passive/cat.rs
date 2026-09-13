use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

use pumpkin_data::entity::{EntityStatus, EntityType};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::meta_data_type::MetaDataType;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tracked_data::TrackedData;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_protocol::java::client::play::Metadata;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::wolf::dye_color_from_item;
use crate::entity::{
    Entity, EntityBase, EntityBaseFuture, NBTStorage, NbtFuture,
    ai::goal::{
        active_target::ActiveTargetGoal, avoid_entity::AvoidEntityGoal, breed::BreedGoal,
        escape_danger::EscapeDangerGoal, follow_owner::FollowOwnerGoal,
        follow_parent::FollowParentGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, swim::SwimGoal, tempt::TemptGoal,
        wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    player::Player,
};

const TEMPT_ITEMS: &[&Item] = &[&Item::COD, &Item::SALMON];

pub struct CatEntity {
    pub mob_entity: MobEntity,
    pub variant: AtomicU8,
    pub collar_color: AtomicU8,
    pub is_tamed: AtomicBool,
    pub is_sitting: AtomicBool,
    pub is_lying: AtomicBool,
    pub owner: Mutex<Option<Uuid>>,
}

impl CatEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let cat = Self {
            mob_entity,
            variant: AtomicU8::new(9), // Default to tabby
            collar_color: AtomicU8::new(14), // Default to red
            is_tamed: AtomicBool::new(false),
            is_sitting: AtomicBool::new(false),
            is_lying: AtomicBool::new(false),
            owner: Mutex::new(None),
        };
        let mob_arc = Arc::new(cat);
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

            goal_selector.add_goal(1, Box::new(SwimGoal::default()));
            goal_selector.add_goal(1, EscapeDangerGoal::new(1.5));
            goal_selector.add_goal(4, Box::new(AvoidEntityGoal::new(&EntityType::PLAYER, 16.0, 0.8, 1.33)));
            goal_selector.add_goal(4, Box::new(TemptGoal::new(0.6, TEMPT_ITEMS)));
            goal_selector.add_goal(5, BreedGoal::new(0.8));
            goal_selector.add_goal(6, FollowOwnerGoal::new(1.0, 5.0, 10.0));
            goal_selector.add_goal(9, Box::new(FollowParentGoal::new(0.8)));
            goal_selector.add_goal(11, Box::new(WanderAroundGoal::new(0.8)));
            goal_selector.add_goal(
                12,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 10.0),
            );
            goal_selector.add_goal(12, Box::new(RandomLookAroundGoal::default()));

            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::RABBIT, false),
            );
            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::TURTLE, false),
            );
        };

        mob_arc
    }

    pub fn collar_color(&self) -> u8 {
        self.collar_color.load(Ordering::Relaxed)
    }

    pub fn set_collar_color(&self, color: u8) {
        self.collar_color.store(color, Ordering::Relaxed);
        self.mob_entity.living_entity.entity.send_meta_data(
            &[Metadata::new(
                TrackedData::COLLAR_COLOR,
                MetaDataType::INT,
                VarInt(color as i32),
            )],
            None,
        );
    }

    pub fn is_tamed(&self) -> bool {
        self.is_tamed.load(Ordering::Relaxed)
    }

    pub fn set_tamed(&self, tamed: bool) {
        self.is_tamed.store(tamed, Ordering::Relaxed);
        self.send_tameable_flags();
    }

    pub fn is_sitting(&self) -> bool {
        self.is_sitting.load(Ordering::Relaxed)
    }

    pub fn set_sitting(&self, sitting: bool) {
        self.is_sitting.store(sitting, Ordering::Relaxed);
        self.send_tameable_flags();
    }

    pub fn is_lying(&self) -> bool {
        self.is_lying.load(Ordering::Relaxed)
    }

    pub fn set_lying(&self, lying: bool) {
        self.is_lying.store(lying, Ordering::Relaxed);
    }

    pub fn tameable_flags(&self) -> u8 {
        let mut flags = 0u8;
        if self.is_sitting() {
            flags |= 1;
        }
        if self.is_tamed() {
            flags |= 4;
        }
        flags
    }

    pub fn send_tameable_flags(&self) {
        let flags = self.tameable_flags();
        self.mob_entity.living_entity.entity.send_meta_data(
            &[Metadata::new(
                TrackedData::TAMEABLE_FLAGS,
                MetaDataType::BYTE,
                flags,
            )],
            None,
        );
    }

    pub fn get_owner_uuid(&self) -> Option<Uuid> {
        *self.owner.try_lock().ok()?
    }

    pub async fn set_owner(&self, owner: Option<Uuid>) {
        *self.owner.lock().await = owner;
        self.mob_entity.living_entity.entity.send_meta_data(
            &[Metadata::new(
                TrackedData::OWNER_UUID,
                MetaDataType::OPTIONAL_UUID,
                owner,
            )],
            None,
        );
    }
}

impl NBTStorage for CatEntity {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.write_nbt(nbt).await;
            let variant_str = match self.variant.load(Ordering::Relaxed) {
                0 => "minecraft:all_black",
                1 => "minecraft:black",
                2 => "minecraft:british_shorthair",
                3 => "minecraft:calico",
                4 => "minecraft:jellie",
                5 => "minecraft:persian",
                6 => "minecraft:ragdoll",
                7 => "minecraft:red",
                8 => "minecraft:siamese",
                10 => "minecraft:white",
                _ => "minecraft:tabby",
            };
            nbt.put_string("variant", variant_str.to_string());
            nbt.put_byte("CollarColor", self.collar_color() as i8);
            nbt.put_bool("Sitting", self.is_sitting());
            nbt.put_bool("Tame", self.is_tamed());
            if let Some(owner) = *self.owner.lock().await {
                nbt.put_uuid("Owner", owner);
            }
        })
    }

    fn read_nbt_non_mut<'a>(&'a self, nbt: &'a NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.read_nbt_non_mut(nbt).await;
            if let Some(variant_str) = nbt.get_string("variant") {
                let variant = match variant_str
                    .strip_prefix("minecraft:")
                    .unwrap_or(variant_str)
                {
                    "all_black" => 0,
                    "black" => 1,
                    "british_shorthair" => 2,
                    "calico" => 3,
                    "jellie" => 4,
                    "persian" => 5,
                    "ragdoll" => 6,
                    "red" => 7,
                    "siamese" => 8,
                    "white" => 10,
                    _ => 9,
                };
                self.variant.store(variant, Ordering::Relaxed);
            }
            if let Some(collar_color) = nbt.get_byte("CollarColor") {
                self.collar_color
                    .store(collar_color as u8, Ordering::Relaxed);
            }
            if let Some(sitting) = nbt.get_bool("Sitting") {
                self.is_sitting.store(sitting, Ordering::Relaxed);
            }
            if let Some(tame) = nbt.get_bool("Tame") {
                self.is_tamed.store(tame, Ordering::Relaxed);
            }
            if let Some(owner) = nbt.get_uuid("Owner") {
                *self.owner.lock().await = Some(owner);
            }
        })
    }
}

impl Mob for CatEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn get_owner_uuid(&self) -> Option<Uuid> {
        self.get_owner_uuid()
    }

    fn is_sitting(&self) -> bool {
        self.is_sitting.load(Ordering::Relaxed)
    }

    fn mob_set_variant_name(&self, name: &str) {
        let variant = match name.strip_prefix("minecraft:").unwrap_or(name) {
            "all_black" => 0,
            "black" => 1,
            "british_shorthair" => 2,
            "calico" => 3,
            "jellie" => 4,
            "persian" => 5,
            "ragdoll" => 6,
            "red" => 7,
            "siamese" => 8,
            "white" => 10,
            _ => 9,
        };
        self.variant.store(variant, Ordering::Relaxed);
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
                &[
                    Metadata::new(
                        TrackedData::CAT_VARIANT,
                        MetaDataType::CAT_VARIANT,
                        VarInt(self.variant.load(Ordering::Relaxed) as i32),
                    ),
                    Metadata::new(
                        TrackedData::COLLAR_COLOR,
                        MetaDataType::INT,
                        VarInt(self.collar_color() as i32),
                    ),
                ],
                None,
            );
            entity.send_meta_data(
                &[Metadata::new(
                    TrackedData::TAMEABLE_FLAGS,
                    MetaDataType::BYTE,
                    self.tameable_flags(),
                )],
                None,
            );
            if let Some(owner) = *self.owner.lock().await {
                entity.send_meta_data(
                    &[Metadata::new(
                        TrackedData::OWNER_UUID,
                        MetaDataType::OPTIONAL_UUID,
                        Some(owner),
                    )],
                    None,
                );
            }
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
            let is_owner = self.get_owner_uuid() == Some(player.gameprofile.id);

            if self.is_tamed() {
                // If dye and owner, dye collar
                if let Some(dye_color) = dye_color_from_item(item_stack.item)
                    && is_owner
                {
                    if self.collar_color() != dye_color {
                        self.set_collar_color(dye_color);
                        item_stack.decrement_unless_creative(player.gamemode.load(), 1);
                        return true;
                    }
                    return true;
                }

                // If food and damaged, heal
                let is_food = item_stack.item.id == Item::COD.id
                    || item_stack.item.id == Item::SALMON.id;
                let current_health = self.mob_entity.living_entity.health.load();
                let max_health = self.mob_entity.living_entity.get_max_health();

                if is_food && current_health < max_health {
                    item_stack.decrement_unless_creative(player.gamemode.load(), 1);
                    self.mob_entity.living_entity.heal(2.0);
                    world.play_sound(
                        Sound::EntityCatEat,
                        SoundCategory::Neutral,
                        &entity.pos.load(),
                    );
                    world.spawn_particle(
                        entity.pos.load() + Vector3::new(0.0, f64::from(entity.height()), 0.0),
                        Vector3::new(0.5, 0.5, 0.5),
                        1.0,
                        5,
                        pumpkin_data::particle::Particle::Heart,
                    );
                    return true;
                }

                // If owner, toggle sitting
                if is_owner {
                    let new_sitting = !self.is_sitting();
                    self.set_sitting(new_sitting);
                    if let Ok(mut nav) = self.mob_entity.navigator.try_lock() {
                        nav.stop();
                    }
                    return true;
                }
            } else {
                // Wild cat: taming with fish
                let is_food = item_stack.item.id == Item::COD.id
                    || item_stack.item.id == Item::SALMON.id;
                if is_food {
                    item_stack.decrement_unless_creative(player.gamemode.load(), 1);
                    world.play_sound(
                        Sound::EntityCatEat,
                        SoundCategory::Neutral,
                        &entity.pos.load(),
                    );
                    let chance = rand::rng().random_range(0..3);
                    if chance == 0 {
                        self.set_tamed(true);
                        self.set_owner(Some(player.gameprofile.id)).await;
                        self.set_sitting(true);
                        world.send_entity_status(entity, EntityStatus::TamingSucceeded, None);
                    } else {
                        world.send_entity_status(entity, EntityStatus::TamingFailed, None);
                    }
                    return true;
                }
            }

            self.mob_entity.mob_interact(player, item_stack).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cat_tameable_flags_bitfield() {
        let tameable_flags = |is_sitting: bool, is_tamed: bool| {
            let mut flags = 0u8;
            if is_sitting {
                flags |= 1;
            }
            if is_tamed {
                flags |= 4;
            }
            flags
        };
        assert_eq!(tameable_flags(false, false), 0);
        assert_eq!(tameable_flags(true, false), 1);
        assert_eq!(tameable_flags(false, true), 4);
        assert_eq!(tameable_flags(true, true), 5);
    }

    #[test]
    fn cat_tracked_data_constants() {
        assert_eq!(TrackedData::CAT_VARIANT.v1_21, 19);
        assert_eq!(TrackedData::COLLAR_COLOR.v1_21, 20);
        assert_eq!(TrackedData::TAMEABLE_FLAGS.v1_21, 17);
    }

    #[test]
    fn cat_nbt_roundtrip() {
        let mut compound = NbtCompound::new();
        compound.put_string("variant", "minecraft:calico".to_string());
        compound.put_byte("CollarColor", 11);
        compound.put_bool("Sitting", true);
        compound.put_bool("Tame", true);

        assert_eq!(compound.get_string("variant"), Some("minecraft:calico"));
        assert_eq!(compound.get_byte("CollarColor"), Some(11));
        assert_eq!(compound.get_bool("Sitting"), Some(true));
        assert_eq!(compound.get_bool("Tame"), Some(true));
    }
}
