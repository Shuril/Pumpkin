use core::f32;

use crate::entity::{Entity, EntityBase, EntityBaseFuture, NBTStorage, living::LivingEntity};
use pumpkin_data::{
    damage::DamageType,
    entity::EntityType,
    meta_data_type::MetaDataType,
    tag::{self, Taggable},
    tracked_data::TrackedData,
};
use pumpkin_protocol::java::client::play::Metadata;
use pumpkin_util::math::vector3::Vector3;

pub struct EndCrystalEntity {
    entity: Entity,
}

impl EndCrystalEntity {
    pub const fn new(entity: Entity) -> Self {
        Self { entity }
    }
}

impl EndCrystalEntity {
    pub fn set_show_bottom(&self, show_bottom: bool) {
        self.entity.send_meta_data(
            &[Metadata::new(
                TrackedData::SHOW_BOTTOM,
                MetaDataType::BOOLEAN,
                show_bottom,
            )],
            None,
        );
    }
}

impl NBTStorage for EndCrystalEntity {}

impl EntityBase for EndCrystalEntity {
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
        damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        source: Option<&'a dyn EntityBase>,
        cause: Option<&'a dyn EntityBase>,
    ) -> EntityBaseFuture<'a, bool> {
        Box::pin(async move {
            if self.entity.is_removed()
                || self.entity.is_invulnerable_to(&damage_type).await
                || cause.or(source).is_some_and(|entity| {
                    entity.get_entity().entity_type.id == EntityType::ENDER_DRAGON.id
                })
            {
                return false;
            }

            let world = self.entity.world.load();
            let position = self.entity.pos.load();
            let block_pos = self.entity.block_pos.load();
            self.entity.remove().await;
            world
                .emit_game_event_from(
                    block_pos,
                    crate::world::game_event::GameEventKind::EntityDie,
                    Some(self.entity.entity_uuid),
                )
                .await;
            if !damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_EXPLOSION) {
                world
                    .emit_game_event(block_pos, crate::world::game_event::GameEventKind::Explode)
                    .await;
                world.explode(position, 6.0).await;
            }

            if let Some(fight) = &world.dragon_fight {
                fight
                    .lock()
                    .await
                    .on_crystal_destroyed(&world, self.entity.entity_uuid)
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
