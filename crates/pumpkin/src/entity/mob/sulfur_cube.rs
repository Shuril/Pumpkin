//! Runtime bridge for the 26.2 sulfur-cube entity.
//!
//! The generated registry already contains the entity and the bucket item. A
//! sulfur cube has the same authoritative movement/collision/splitting shape
//! as a magma cube (both are `AbstractCubeMob`s), so reuse the battle-tested
//! cube state machine while keeping the entity type, NBT and network ID as
//! `minecraft:sulfur_cube`. This keeps bucket-spawned cubes alive and
//! persistent instead of falling back to a generic `LivingEntity`.

use std::sync::Arc;

use crate::entity::mob::{Mob, MobEntity, slime::SlimeEntity};
use crate::entity::{Entity, NBTStorage};

pub struct SulfurCubeEntity {
    pub slime: Arc<SlimeEntity>,
}

impl SulfurCubeEntity {
    #[must_use]
    pub fn new(entity: Entity) -> Arc<Self> {
        let slime = SlimeEntity::new(entity);
        // 26.2 sulfur cubes have only the small/large (1/2) size range.  A
        // bucket always yields the canonical large size; persisted Size NBT is
        // still read by the delegated SlimeEntity implementation afterwards.
        slime.set_size(2, true);
        Arc::new(Self { slime })
    }
}

impl NBTStorage for SulfurCubeEntity {}

impl Mob for SulfurCubeEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        self.slime.get_mob_entity()
    }

    fn mob_tick<'a>(
        &'a self,
        caller: &'a Arc<dyn crate::entity::EntityBase>,
    ) -> crate::entity::EntityBaseFuture<'a, ()> {
        self.slime.mob_tick(caller)
    }

    fn post_tick(&self) -> crate::entity::EntityBaseFuture<'_, ()> {
        self.slime.post_tick()
    }

    fn mob_player_collision<'a>(
        &'a self,
        player: &'a Arc<crate::entity::player::Player>,
    ) -> crate::entity::EntityBaseFuture<'a, ()> {
        self.slime.mob_player_collision(player)
    }
}
