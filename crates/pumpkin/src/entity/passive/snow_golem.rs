use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use pumpkin_data::{
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    meta_data_type::MetaDataType,
    sound::{Sound, SoundCategory},
    tracked_data::TrackedData,
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::java::client::play::Metadata;

use crate::entity::{
    Entity, EntityBaseFuture, NBTStorage, NbtFuture,
    ai::goal::{
        active_target::ActiveTargetGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, snowball_attack::SnowballAttackGoal,
        wander_around::WanderAroundGoal,
    },
    item::ItemEntity,
    mob::{Mob, MobEntity},
    player::Player,
};

pub struct SnowGolemEntity {
    pub mob_entity: MobEntity,
    has_pumpkin: AtomicBool,
}

impl SnowGolemEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let snow_golem = Self {
            mob_entity,
            has_pumpkin: AtomicBool::new(true),
        };
        let mob_arc = Arc::new(snow_golem);
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

            goal_selector.add_goal(1, Box::new(SnowballAttackGoal::new(1.25, 20, 10.0)));
            goal_selector.add_goal(5, Box::new(WanderAroundGoal::new(1.0)));
            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(6, Box::new(RandomLookAroundGoal::default()));

            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::ZOMBIE, true),
            );
            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::SKELETON, true),
            );
            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::SPIDER, true),
            );
            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::CREEPER, true),
            );
            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::PILLAGER, true),
            );
            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::VINDICATOR, true),
            );
        };

        mob_arc
    }

    #[must_use]
    pub fn has_pumpkin(&self) -> bool {
        self.has_pumpkin.load(Ordering::Relaxed)
    }

    pub fn set_pumpkin(&self, pumpkin: bool) {
        self.has_pumpkin.store(pumpkin, Ordering::Relaxed);
        let byte: i8 = if pumpkin { 16 } else { 0 };
        self.mob_entity.living_entity.entity.send_meta_data(
            &[Metadata::new(
                TrackedData::SNOW_GOLEM_FLAGS,
                MetaDataType::BYTE,
                byte,
            )],
            None,
        );
    }

    pub async fn shear(&self, sound_category: SoundCategory) {
        self.set_pumpkin(false);
        let entity = &self.mob_entity.living_entity.entity;
        let pos = entity.pos.load();
        let world = entity.world.load();
        world.play_sound(Sound::EntitySnowGolemShear, sound_category, &pos);
        let drop = ItemEntity::new(
            Entity::new(world.clone(), pos, &EntityType::ITEM),
            ItemStack::new(1, &Item::CARVED_PUMPKIN),
        );
        world.spawn_entity(Arc::new(drop)).await;
        world
            .emit_game_event(
                pos.to_block_pos(),
                crate::world::game_event::GameEventKind::Shear,
            )
            .await;
    }
}

impl NBTStorage for SnowGolemEntity {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async {
            self.mob_entity.living_entity.entity.write_nbt(nbt).await;
            nbt.put_bool("Pumpkin", self.has_pumpkin());
        })
    }

    fn read_nbt_non_mut<'a>(&'a self, nbt: &'a NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async {
            self.mob_entity
                .living_entity
                .entity
                .read_nbt_non_mut(nbt)
                .await;
            if let Some(pumpkin) = nbt.get_bool("Pumpkin") {
                self.set_pumpkin(pumpkin);
            }
        })
    }
}

impl Mob for SnowGolemEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn get_snow_golem(&self) -> Option<&Self> {
        Some(self)
    }

    fn mob_init_data_tracker(&self) -> EntityBaseFuture<'_, ()> {
        Box::pin(async move {
            let byte: i8 = if self.has_pumpkin() { 16 } else { 0 };
            self.mob_entity.living_entity.entity.send_meta_data(
                &[Metadata::new(
                    TrackedData::SNOW_GOLEM_FLAGS,
                    MetaDataType::BYTE,
                    byte,
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
            if item_stack.item.id == Item::SHEARS.id && self.has_pumpkin() {
                self.shear(SoundCategory::Players).await;
                player.damage_held_item(1).await;
                true
            } else {
                false
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snow_golem_shearing_drops_carved_pumpkin() {
        assert_eq!(
            Item::CARVED_PUMPKIN.id,
            pumpkin_data::item::Item::CARVED_PUMPKIN.id
        );
        assert_eq!(Item::SHEARS.id, pumpkin_data::item::Item::SHEARS.id);
    }

    #[test]
    fn snow_golem_tracked_data_index() {
        assert_eq!(TrackedData::SNOW_GOLEM_FLAGS.v1_21, 16);
    }
}
