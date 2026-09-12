use std::sync::Arc;

use pumpkin_data::{damage::DamageType, item::Item, item_stack::ItemStack};

use crate::entity::{
    Entity, EntityBase, EntityBaseFuture, NBTStorage,
    mob::{Mob, MobEntity, skeleton::SkeletonEntityBase},
};

pub struct WitherSkeletonEntity {
    entity: Arc<SkeletonEntityBase>,
}

impl WitherSkeletonEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = SkeletonEntityBase::new(entity);
        let skeleton = Self { entity };
        Arc::new(skeleton)
    }
}

impl NBTStorage for WitherSkeletonEntity {}

impl Mob for WitherSkeletonEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    fn mob_drop_custom_death_loot<'a>(
        &'a self,
        _damage_type: DamageType,
        source: Option<&'a dyn EntityBase>,
        cause: Option<&'a dyn EntityBase>,
    ) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move {
            let entity = &self.entity.mob_entity.living_entity.entity;
            let world = entity.world.load();
            if !world.level_info.load().game_rules.mob_drops {
                return;
            }

            let killer = cause.or(source);
            if let Some(killer) = killer
                && let Some(creeper) = killer.get_mob().and_then(|m| m.get_creeper())
                && creeper.can_drop_mob_head()
            {
                creeper.increase_dropped_mob_heads();
                world
                    .drop_stack(
                        &entity.block_pos.load(),
                        ItemStack::new(1, &Item::WITHER_SKELETON_SKULL),
                    )
                    .await;
            }
        })
    }
}
