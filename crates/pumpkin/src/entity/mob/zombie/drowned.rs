use std::sync::Arc;

use crate::entity::mob::zombie::ZombieEntityBase;
use crate::entity::{
    Entity, NBTStorage, NbtFuture,
    mob::{Mob, MobEntity},
};
use pumpkin_nbt::compound::NbtCompound;

pub struct DrownedEntity {
    entity: Arc<ZombieEntityBase>,
}

impl DrownedEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = ZombieEntityBase::new(entity);
        let zombie = Self { entity };
        Arc::new(zombie)
    }
}

impl NBTStorage for DrownedEntity {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        self.entity.write_nbt(nbt)
    }

    fn read_nbt_non_mut<'a>(&'a self, nbt: &'a NbtCompound) -> NbtFuture<'a, ()> {
        self.entity.read_nbt_non_mut(nbt)
    }
}

impl Mob for DrownedEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    fn can_break_doors(&self) -> bool {
        self.entity.can_break_doors()
    }

    fn set_can_break_doors(&self, can_break: bool) {
        self.entity.set_can_break_doors(can_break);
    }

    fn mob_drop_custom_death_loot<'a>(
        &'a self,
        damage_type: pumpkin_data::damage::DamageType,
        source: Option<&'a dyn crate::entity::EntityBase>,
        cause: Option<&'a dyn crate::entity::EntityBase>,
    ) -> crate::entity::EntityBaseFuture<'a, ()> {
        self.entity.mob_drop_custom_death_loot(damage_type, source, cause)
    }
}
