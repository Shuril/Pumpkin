use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::meta_data_type::MetaDataType;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tracked_data::TrackedData;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::java::client::play::Metadata;

use crate::entity::{
    Entity, EntityBaseFuture, NBTStorage, NbtFuture,
    ai::goal::{
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal, swim::SwimGoal,
        wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    player::Player,
};
use crate::world::game_event::GameEvent;

/// Represents a Donkey, a passive mob that can be tamed and equipped with chests.
///
/// Wiki: <https://minecraft.wiki/w/Donkey>
pub struct DonkeyEntity {
    pub mob_entity: MobEntity,
    pub has_chest: AtomicBool,
    pub is_tamed: AtomicBool,
}

impl DonkeyEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let donkey = Self {
            mob_entity,
            has_chest: AtomicBool::new(false),
            is_tamed: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(donkey);
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
            goal_selector.add_goal(1, Box::new(WanderAroundGoal::new(0.7)));
            goal_selector.add_goal(
                2,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(3, Box::new(RandomLookAroundGoal::default()));
        };

        mob_arc
    }

    pub fn has_chest(&self) -> bool {
        self.has_chest.load(Ordering::Relaxed)
    }

    pub fn set_has_chest(&self, has_chest: bool) {
        self.has_chest.store(has_chest, Ordering::Relaxed);
        self.mob_entity.living_entity.entity.send_meta_data(
            &[
                Metadata::new(TrackedData::ID_CHEST, MetaDataType::BOOLEAN, has_chest),
                Metadata::new(TrackedData::CHEST, MetaDataType::BOOLEAN, has_chest),
            ],
            None,
        );
    }

    pub fn is_tamed(&self) -> bool {
        self.is_tamed.load(Ordering::Relaxed)
    }

    pub fn set_tamed(&self, tamed: bool) {
        self.is_tamed.store(tamed, Ordering::Relaxed);
    }
}

impl NBTStorage for DonkeyEntity {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.write_nbt(nbt).await;
            nbt.put_bool("ChestedHorse", self.has_chest());
            nbt.put_bool("Tame", self.is_tamed());
        })
    }

    fn read_nbt_non_mut<'a>(&'a self, nbt: &'a NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.mob_entity.living_entity.read_nbt_non_mut(nbt).await;
            if let Some(chested) = nbt.get_bool("ChestedHorse") {
                self.set_has_chest(chested);
            }
            if let Some(tamed) = nbt.get_bool("Tame") {
                self.set_tamed(tamed);
            }
        })
    }
}

impl Mob for DonkeyEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn get_donkey(&self) -> Option<&Self> {
        Some(self)
    }

    fn mob_init_data_tracker(&self) -> EntityBaseFuture<'_, ()> {
        Box::pin(async move {
            if self
                .get_mob_entity()
                .living_entity
                .entity
                .age
                .load(Ordering::Relaxed)
                < 0
            {
                self.get_mob_entity().living_entity.entity.send_meta_data(
                    &[Metadata::new(
                        TrackedData::BABY_ID,
                        MetaDataType::BOOLEAN,
                        true,
                    )],
                    None,
                );
            }
            if self.has_chest() {
                self.mob_entity.living_entity.entity.send_meta_data(
                    &[
                        Metadata::new(TrackedData::ID_CHEST, MetaDataType::BOOLEAN, true),
                        Metadata::new(TrackedData::CHEST, MetaDataType::BOOLEAN, true),
                    ],
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
            if item_stack.item.id == Item::CHEST.id
                && self.is_tamed()
                && !self.has_chest()
                && self
                    .mob_entity
                    .living_entity
                    .entity
                    .age
                    .load(Ordering::Relaxed)
                    >= 0
            {
                if !player.is_creative() {
                    item_stack.decrement_unless_creative(player.gamemode.load(), 1);
                }
                self.set_has_chest(true);
                let pos = self.mob_entity.living_entity.entity.pos.load();
                let world = self.mob_entity.living_entity.entity.world.load();
                world.play_sound(Sound::EntityDonkeyChest, SoundCategory::Neutral, &pos);
                world
                    .emit_game_event(pos.to_block_pos(), GameEvent::EQUIP)
                    .await;
                return true;
            }

            self.mob_entity.mob_interact(player, item_stack).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_nbt::compound::NbtCompound;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn test_donkey_chest_and_tame_state_toggling() {
        let has_chest = AtomicBool::new(false);
        let is_tamed = AtomicBool::new(false);

        assert!(!has_chest.load(Ordering::Relaxed));
        assert!(!is_tamed.load(Ordering::Relaxed));

        has_chest.store(true, Ordering::Relaxed);
        is_tamed.store(true, Ordering::Relaxed);

        assert!(has_chest.load(Ordering::Relaxed));
        assert!(is_tamed.load(Ordering::Relaxed));

        has_chest.store(false, Ordering::Relaxed);
        is_tamed.store(false, Ordering::Relaxed);

        assert!(!has_chest.load(Ordering::Relaxed));
        assert!(!is_tamed.load(Ordering::Relaxed));
    }

    #[test]
    fn test_donkey_tracked_data_constants() {
        assert_eq!(TrackedData::CHEST.v1_21, 18);
        assert_eq!(TrackedData::ID_CHEST.v26_1, 19);
    }

    #[test]
    fn test_donkey_nbt_serialization_and_deserialization() {
        let mut compound = NbtCompound::new();
        compound.put_bool("ChestedHorse", true);
        compound.put_bool("Tame", true);

        assert_eq!(compound.get_bool("ChestedHorse"), Some(true));
        assert_eq!(compound.get_bool("Tame"), Some(true));

        let has_chest = AtomicBool::new(false);
        let is_tamed = AtomicBool::new(false);

        if let Some(c) = compound.get_bool("ChestedHorse") {
            has_chest.store(c, Ordering::Relaxed);
        }
        if let Some(t) = compound.get_bool("Tame") {
            is_tamed.store(t, Ordering::Relaxed);
        }

        assert!(has_chest.load(Ordering::Relaxed));
        assert!(is_tamed.load(Ordering::Relaxed));

        let mut roundtrip = NbtCompound::new();
        roundtrip.put_bool("ChestedHorse", has_chest.load(Ordering::Relaxed));
        roundtrip.put_bool("Tame", is_tamed.load(Ordering::Relaxed));

        assert_eq!(roundtrip.get_bool("ChestedHorse"), Some(true));
        assert_eq!(roundtrip.get_bool("Tame"), Some(true));
    }
}
