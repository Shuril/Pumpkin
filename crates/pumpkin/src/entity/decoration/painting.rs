use core::f32;
use std::sync::atomic::Ordering;

use crate::entity::{
    Entity, EntityBase, EntityBaseFuture, NBTStorage, NbtFuture, living::LivingEntity,
};
use pumpkin_data::damage::DamageType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::vector3::Vector3;

pub struct PaintingEntity {
    entity: Entity,
}

impl PaintingEntity {
    pub const fn new(entity: Entity) -> Self {
        Self { entity }
    }
}

impl NBTStorage for PaintingEntity {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.entity.write_nbt(nbt).await;
            nbt.put_byte("facing", self.entity.data.load(Ordering::Relaxed) as i8);
        })
    }

    fn read_nbt_non_mut<'a>(&'a self, nbt: &'a NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async {
            self.entity.read_nbt_non_mut(nbt).await;
            let facing = nbt.get_byte("facing").unwrap_or(3);
            self.entity.data.store(facing as i32, Ordering::Relaxed);
        })
    }
}

impl EntityBase for PaintingEntity {
    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn damage_with_context<'a>(
        &'a self,
        _caller: &'a dyn EntityBase,
        _amount: f32,
        _damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        source: Option<&'a dyn EntityBase>,
        cause: Option<&'a dyn EntityBase>,
    ) -> EntityBaseFuture<'a, bool> {
        Box::pin(async move {
            if self.entity.is_removed() {
                return false;
            }

            let world = self.entity.world.load();
            let caused_by_player = cause
                .or(source)
                .is_some_and(|entity| entity.get_player().is_some());
            let drops = world.level_info.load().game_rules.entity_drops;
            world.play_sound_fine(
                Sound::EntityPaintingBreak,
                SoundCategory::Neutral,
                &self.entity.pos.load(),
                1.0,
                1.0,
            );
            world
                .emit_game_event_from(
                    self.entity.block_pos.load(),
                    crate::world::game_event::GameEventKind::EntityDie,
                    Some(self.entity.entity_uuid),
                )
                .await;
            self.entity.remove().await;

            if drops
                && !(caused_by_player
                    && cause.or(source).is_some_and(|entity| {
                        entity
                            .get_player()
                            .is_some_and(|player| player.is_creative())
                    }))
            {
                world
                    .drop_stack(
                        &self.entity.block_pos.load(),
                        ItemStack::new(1, &Item::PAINTING),
                    )
                    .await;
            }
            true
        })
    }

    fn as_nbt_storage(&self) -> &dyn NBTStorage {
        self
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}
