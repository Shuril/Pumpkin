use crate::entity::mob::zombie::ZombieEntityBase;
use crate::entity::mob::{Mob, MobEntity};
use crate::entity::{Entity, NBTStorage, NbtFuture};
use pumpkin_nbt::compound::NbtCompound;
use std::sync::Arc;

pub struct ZombieVillagerEntity {
    pub mob_entity: Arc<ZombieEntityBase>,
}

impl ZombieVillagerEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = ZombieEntityBase::new(entity);
        let zombie = Self { mob_entity };
        Arc::new(zombie)
    }
}

impl NBTStorage for ZombieVillagerEntity {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        self.mob_entity.write_nbt(nbt)
    }

    fn read_nbt_non_mut<'a>(&'a self, nbt: &'a NbtCompound) -> NbtFuture<'a, ()> {
        self.mob_entity.read_nbt_non_mut(nbt)
    }
}

impl Mob for ZombieVillagerEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity.mob_entity
    }

    fn can_break_doors(&self) -> bool {
        self.mob_entity.can_break_doors()
    }

    fn set_can_break_doors(&self, can_break: bool) {
        self.mob_entity.set_can_break_doors(can_break);
    }

    fn mob_drop_custom_death_loot<'a>(
        &'a self,
        damage_type: pumpkin_data::damage::DamageType,
        source: Option<&'a dyn crate::entity::EntityBase>,
        cause: Option<&'a dyn crate::entity::EntityBase>,
    ) -> crate::entity::EntityBaseFuture<'a, ()> {
        self.mob_entity.mob_drop_custom_death_loot(damage_type, source, cause)
    }
}
