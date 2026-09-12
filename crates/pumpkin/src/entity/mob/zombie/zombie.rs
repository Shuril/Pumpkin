use crate::entity::mob::zombie::ZombieEntityBase;
use crate::entity::mob::{Mob, MobEntity};
use crate::entity::{Entity, NBTStorage, NbtFuture};
use pumpkin_nbt::compound::NbtCompound;
use std::sync::Arc;

pub struct ZombieEntity {
    entity: Arc<ZombieEntityBase>,
}

impl ZombieEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = ZombieEntityBase::new(entity);
        let zombie = Self { entity };
        Arc::new(zombie)
    }
}

impl NBTStorage for ZombieEntity {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        self.entity.write_nbt(nbt)
    }

    fn read_nbt_non_mut<'a>(&'a self, nbt: &'a NbtCompound) -> NbtFuture<'a, ()> {
        self.entity.read_nbt_non_mut(nbt)
    }
}

impl Mob for ZombieEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    fn can_break_doors(&self) -> bool {
        self.entity.can_break_doors()
    }

    fn set_can_break_doors(&self, can_break: bool) {
        self.entity.set_can_break_doors(can_break);
    }
}
