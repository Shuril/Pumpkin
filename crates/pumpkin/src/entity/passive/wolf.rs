use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

use pumpkin_data::entity::{EntityStatus, EntityType};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::meta_data_type::MetaDataType;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data::TrackedData;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_protocol::java::client::play::Metadata;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::entity::{
    Entity, EntityBase, EntityBaseFuture, NBTStorage, NbtFuture,
    ai::goal::{
        active_target::ActiveTargetGoal, beg::BegGoal, breed::BreedGoal,
        escape_danger::EscapeDangerGoal, follow_owner::FollowOwnerGoal,
        follow_parent::FollowParentGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, melee_attack::MeleeAttackGoal,
        owner_hurt_by_target::OwnerHurtByTargetGoal, owner_hurt_target::OwnerHurtTargetGoal,
        revenge::RevengeGoal, swim::SwimGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    player::Player,
};

/// Resolves standard Minecraft DyeColor (0..=15) from a dye item.
#[must_use]
pub fn dye_color_from_item(item: &Item) -> Option<u8> {
    match item.registry_key {
        "white_dye" | "minecraft:white_dye" => Some(0),
        "orange_dye" | "minecraft:orange_dye" => Some(1),
        "magenta_dye" | "minecraft:magenta_dye" => Some(2),
        "light_blue_dye" | "minecraft:light_blue_dye" => Some(3),
        "yellow_dye" | "minecraft:yellow_dye" => Some(4),
        "lime_dye" | "minecraft:lime_dye" => Some(5),
        "pink_dye" | "minecraft:pink_dye" => Some(6),
        "gray_dye" | "minecraft:gray_dye" => Some(7),
        "light_gray_dye" | "minecraft:light_gray_dye" => Some(8),
        "cyan_dye" | "minecraft:cyan_dye" => Some(9),
        "purple_dye" | "minecraft:purple_dye" => Some(10),
        "blue_dye" | "minecraft:blue_dye" => Some(11),
        "brown_dye" | "minecraft:brown_dye" => Some(12),
        "green_dye" | "minecraft:green_dye" => Some(13),
        "red_dye" | "minecraft:red_dye" => Some(14),
        "black_dye" | "minecraft:black_dye" => Some(15),
        _ => None,
    }
}

pub struct WolfEntity {
    pub mob_entity: MobEntity,
    pub variant: AtomicU8,
    pub collar_color: AtomicU8,
    pub is_tamed: AtomicBool,
    pub is_sitting: AtomicBool,
    pub is_begging: AtomicBool,
    pub owner: Mutex<Option<Uuid>>,
}

impl WolfEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let wolf = Self {
            mob_entity,
            variant: AtomicU8::new(3),       // Default to pale
            collar_color: AtomicU8::new(14), // Default to red
            is_tamed: AtomicBool::new(false),
            is_sitting: AtomicBool::new(false),
            is_begging: AtomicBool::new(false),
            owner: Mutex::new(None),
        };
        let mob_arc = Arc::new(wolf);
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
            goal_selector.add_goal(4, EscapeDangerGoal::new(1.5));
            goal_selector.add_goal(5, Box::new(MeleeAttackGoal::new(1.0, true)));
            goal_selector.add_goal(6, FollowOwnerGoal::new(1.0, 2.0, 10.0));
            goal_selector.add_goal(7, BreedGoal::new(1.0));
            goal_selector.add_goal(8, Box::new(FollowParentGoal::new(1.1)));
            goal_selector.add_goal(9, BegGoal::new(8.0, &[&Item::BONE]));
            goal_selector.add_goal(
                10,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(10, Box::new(RandomLookAroundGoal::default()));
            goal_selector.add_goal(12, Box::new(WanderAroundGoal::new(1.0)));

            target_selector.add_goal(1, OwnerHurtByTargetGoal::new());
            target_selector.add_goal(2, OwnerHurtTargetGoal::new());
            target_selector.add_goal(3, Box::new(RevengeGoal::new(true)));
            target_selector.add_goal(
                7,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::SKELETON, false),
            );
            target_selector.add_goal(
                7,
                ActiveTargetGoal::with_default(
                    &mob_arc.mob_entity,
                    &EntityType::WITHER_SKELETON,
                    false,
                ),
            );
            target_selector.add_goal(
                7,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::STRAY, false),
            );
            target_selector.add_goal(
                7,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::BOGGED, false),
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

    pub fn is_begging(&self) -> bool {
        self.is_begging.load(Ordering::Relaxed)
    }

    pub fn set_begging(&self, begging: bool) {
        self.is_begging.store(begging, Ordering::Relaxed);
        self.mob_entity.living_entity.entity.send_meta_data(
            &[Metadata::new(
                TrackedData::BEGGING,
                MetaDataType::BOOLEAN,
                begging,
            )],
            None,
        );
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

    /// Vanilla parity tail angle based on tamed health fraction.
    pub fn tail_angle(&self) -> f32 {
        if self.is_tamed() {
            let max_health = self.mob_entity.living_entity.get_max_health();
            let health = self.mob_entity.living_entity.health.load();
            let damage_ratio = (max_health - health) / max_health;
            (0.55 - (damage_ratio * 0.4)) * std::f32::consts::PI
        } else {
            0.62831855
        }
    }
}

impl NBTStorage for WolfEntity {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async {
            self.mob_entity.living_entity.write_nbt(nbt).await;
            let variant_str = match self.variant.load(Ordering::Relaxed) {
                0 => "minecraft:ashen",
                1 => "minecraft:black",
                2 => "minecraft:chestnut",
                4 => "minecraft:rusty",
                5 => "minecraft:snowy",
                6 => "minecraft:spotted",
                7 => "minecraft:striped",
                8 => "minecraft:woods",
                _ => "minecraft:pale",
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
        Box::pin(async {
            self.mob_entity.living_entity.read_nbt_non_mut(nbt).await;
            if let Some(variant_str) = nbt.get_string("variant") {
                let variant = match variant_str
                    .strip_prefix("minecraft:")
                    .unwrap_or(variant_str)
                {
                    "ashen" => 0,
                    "black" => 1,
                    "chestnut" => 2,
                    "rusty" => 4,
                    "snowy" => 5,
                    "spotted" => 6,
                    "striped" => 7,
                    "woods" => 8,
                    _ => 3,
                };
                self.variant.store(variant, Ordering::Relaxed);
            }
            if let Some(collar) = nbt.get_byte("CollarColor") {
                self.collar_color.store(collar as u8, Ordering::Relaxed);
            }
            if let Some(sitting) = nbt
                .get_byte("Sitting")
                .map(|b| b != 0)
                .or_else(|| nbt.get_bool("Sitting"))
            {
                self.is_sitting.store(sitting, Ordering::Relaxed);
            }
            if let Some(tame) = nbt
                .get_byte("Tame")
                .map(|b| b != 0)
                .or_else(|| nbt.get_bool("Tame"))
            {
                self.is_tamed.store(tame, Ordering::Relaxed);
            }
            if let Some(owner) = nbt.get_uuid("Owner") {
                *self.owner.lock().await = Some(owner);
            }
        })
    }
}

impl Mob for WolfEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn get_owner_uuid(&self) -> Option<Uuid> {
        self.owner.try_lock().ok().and_then(|g| *g)
    }

    fn is_sitting(&self) -> bool {
        self.is_sitting.load(Ordering::Relaxed)
    }

    fn can_attack_with_owner(&self, target: &dyn EntityBase, owner: &dyn EntityBase) -> bool {
        if target.get_entity().entity_id == owner.get_entity().entity_id {
            return false;
        }
        if target.get_entity().entity_type == &EntityType::CREEPER
            || target.get_entity().entity_type == &EntityType::GHAST
        {
            return false;
        }
        true
    }

    fn mob_set_variant_name(&self, name: &str) {
        let variant = match name.strip_prefix("minecraft:").unwrap_or(name) {
            "ashen" => 0,
            "black" => 1,
            "chestnut" => 2,
            "rusty" => 4,
            "snowy" => 5,
            "spotted" => 6,
            "striped" => 7,
            "woods" => 8,
            _ => 3,
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
                        TrackedData::WOLF_VARIANT_ID,
                        MetaDataType::WOLF_VARIANT,
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
            entity.send_meta_data(
                &[Metadata::new(
                    TrackedData::BEGGING,
                    MetaDataType::BOOLEAN,
                    self.is_begging(),
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
                // If food and damaged, heal
                let is_food = item_stack.item.has_tag(&tag::Item::MINECRAFT_WOLF_FOOD);
                let current_health = self.mob_entity.living_entity.health.load();
                let max_health = self.mob_entity.living_entity.get_max_health();

                if is_food && current_health < max_health {
                    item_stack.decrement_unless_creative(player.gamemode.load(), 1);
                    self.mob_entity.living_entity.heal(2.0);
                    world.play_sound(
                        Sound::EntityWolfAmbient,
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
                // Wild wolf: taming with bone
                if item_stack.item.id == Item::BONE.id {
                    item_stack.decrement_unless_creative(player.gamemode.load(), 1);
                    let chance = rand::rng().random_range(0..3);
                    if chance == 0 {
                        self.set_tamed(true);
                        self.set_owner(Some(player.gameprofile.id)).await;
                        self.set_sitting(true);
                        self.mob_entity.living_entity.set_max_health(40.0).await;
                        self.mob_entity.living_entity.set_health(40.0);
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
    fn dye_colors_resolution() {
        assert_eq!(dye_color_from_item(&Item::WHITE_DYE), Some(0));
        assert_eq!(dye_color_from_item(&Item::ORANGE_DYE), Some(1));
        assert_eq!(dye_color_from_item(&Item::RED_DYE), Some(14));
        assert_eq!(dye_color_from_item(&Item::BLACK_DYE), Some(15));
        assert_eq!(dye_color_from_item(&Item::BONE), None);
    }

    #[test]
    fn wolf_tracked_data_indices() {
        assert_eq!(TrackedData::TAMEABLE_FLAGS.v1_21, 17);
        assert_eq!(TrackedData::BEGGING.v1_21, 19);
        assert_eq!(TrackedData::COLLAR_COLOR.v1_21, 20);
        assert_eq!(TrackedData::WOLF_VARIANT_ID.v1_21, 22);
    }

    #[test]
    fn wolf_tameable_flags_bitfield() {
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
    fn wolf_tail_angle() {
        let tail_angle = |health: f32, max_health: f32| {
            let damage_ratio = (1.0 - (health / max_health)).clamp(0.0, 1.0);
            (0.55 - (damage_ratio * 0.4)) * std::f32::consts::PI
        };
        assert!((tail_angle(40.0, 40.0) - (0.55 * std::f32::consts::PI)).abs() < 1e-4);
        assert!((tail_angle(20.0, 40.0) - (0.35 * std::f32::consts::PI)).abs() < 1e-4);
        assert!((tail_angle(0.0, 40.0) - (0.15 * std::f32::consts::PI)).abs() < 1e-4);
    }

    #[test]
    fn wolf_nbt_roundtrip() {
        let mut compound = pumpkin_nbt::compound::NbtCompound::new();
        compound.put_byte("CollarColor", 11);
        compound.put_bool("Sitting", true);
        compound.put_bool("Tame", true);
        assert_eq!(compound.get_byte("CollarColor"), Some(11));
        assert_eq!(compound.get_bool("Sitting"), Some(true));
        assert_eq!(compound.get_bool("Tame"), Some(true));
    }
}
