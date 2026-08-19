use pumpkin_data::BlockStateId;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::fluid::Fluid;
use pumpkin_data::meta_data_type::MetaDataType;
use pumpkin_data::{Block, tracked_data::TrackedData};
use pumpkin_protocol::java::client::play::Metadata;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, atomic::Ordering};

use crate::{
    entity::{Entity, EntityBase, EntityBaseFuture, NBTStorage, living::LivingEntity},
    server::Server,
    world::World,
};

pub struct FallingEntity {
    entity: Entity,
    block_state_id: BlockStateId,
}

impl FallingEntity {
    pub const fn new(entity: Entity, block_state_id: BlockStateId) -> Self {
        Self {
            entity,
            block_state_id,
        }
    }

    /// Replaced the current Block and Spawns a new Falling one
    pub async fn replace_spawn(world: &Arc<World>, position: BlockPos, block_state: BlockStateId) {
        // Vanilla FallingBlockEntity.fall replaces the source with
        // `state.getFluidState().createLegacyBlock()`, not unconditional air.
        // In particular, a waterlogged block leaves a water source behind
        // while its dry block state is carried by the entity.
        let fluid = world.get_fluid(&position);
        let replacement = if fluid.id == Fluid::WATER.id || fluid.id == Fluid::FLOWING_WATER.id {
            Block::WATER.default_state.id
        } else if fluid.id == Fluid::LAVA.id || fluid.id == Fluid::FLOWING_LAVA.id {
            Block::LAVA.default_state.id
        } else {
            Block::AIR.default_state.id
        };
        world
            .set_block_state(&position, replacement, BlockFlags::NOTIFY_ALL)
            .await;

        // FallingBlockEntity.fall carries the block's dry state.  The source
        // fluid is represented by `replacement` above; retaining
        // `waterlogged=true` on the entity would make a later landing create a
        // second fluid source and diverge from vanilla.
        let carried_state = Block::from_state_id(block_state)
            .without_waterlogged(block_state)
            .map_or(block_state, |state| state.id);

        let position = position.0.to_f64().add_raw(0.5, 0.0, 0.5);
        let entity = Entity::new(world.clone(), position, &EntityType::FALLING_BLOCK);
        entity
            .data
            .store(i32::from(carried_state.as_u16()), Ordering::Relaxed);
        let entity = Arc::new(Self::new(entity, carried_state));
        world.spawn_entity(entity).await;
    }
}

impl NBTStorage for FallingEntity {}

impl EntityBase for FallingEntity {
    fn tick<'a>(
        &'a self,
        caller: &'a Arc<dyn EntityBase>,
        server: &'a Server,
    ) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move {
            let entity = &self.entity;
            entity.tick(caller, server).await;

            let original_velo = entity.velocity.load();
            let mut velo = original_velo;
            velo.y -= self.get_gravity();

            entity.velocity.store(velo);

            entity.move_entity(caller, velo).await;
            entity.tick_block_collisions(caller, server).await;
            if entity.on_ground.load(Ordering::Relaxed) {
                entity.velocity.store(velo.multiply(0.7, -0.5, 0.7));
                entity
                    .world
                    .load()
                    .set_block_state(
                        &self.entity.block_pos.load(),
                        self.block_state_id,
                        BlockFlags::NOTIFY_ALL,
                    )
                    .await;
                entity.remove().await;
            }

            entity.velocity.store(velo.multiply(0.98, 0.98, 0.98));

            if entity.velocity_dirty.swap(false, Ordering::SeqCst) {
                entity.send_pos_rot();
                entity.send_velocity();
            }
        })
    }

    fn init_data_tracker(&self) -> EntityBaseFuture<'_, ()> {
        Box::pin(async move {
            self.entity.send_meta_data(
                &[Metadata::new(
                    TrackedData::START_POS,
                    MetaDataType::BLOCK_POS,
                    self.entity.block_pos.load(),
                )],
                None,
            );
        })
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn as_nbt_storage(&self) -> &dyn NBTStorage {
        self
    }

    fn damage<'a>(
        &'a self,
        _caller: &'a dyn EntityBase,
        _amount: f32,
        _damage_type: DamageType,
    ) -> EntityBaseFuture<'a, bool> {
        Box::pin(async move { false })
    }

    fn get_gravity(&self) -> f64 {
        0.04
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}
