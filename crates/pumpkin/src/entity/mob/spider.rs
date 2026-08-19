use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Weak};

use pumpkin_data::entity::EntityType;
use pumpkin_data::meta_data_type::MetaDataType;
use pumpkin_data::tracked_data::TrackedData;
use pumpkin_protocol::java::client::play::Metadata;

use crate::entity::EntityBaseFuture;
use crate::entity::{
    Entity, NBTStorage,
    ai::goal::{
        active_target::ActiveTargetGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, melee_attack::MeleeAttackGoal, revenge::RevengeGoal,
        swim::SwimGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
};

pub struct SpiderEntity {
    pub mob_entity: MobEntity,
}

impl SpiderEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let spider = Self { mob_entity };
        let mob_arc = Arc::new(spider);
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

            goal_selector.add_goal(1, Box::new(SwimGoal::default()));
            // TODO: SpiderAttackGoal for jumping
            goal_selector.add_goal(3, Box::new(MeleeAttackGoal::new(1.0, false)));
            goal_selector.add_goal(5, Box::new(WanderAroundGoal::new(0.8)));
            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(6, Box::new(RandomLookAroundGoal::default()));

            target_selector.add_goal(1, Box::new(RevengeGoal::new(true)));
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::PLAYER, true),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::IRON_GOLEM, true),
            );
        };

        mob_arc
    }
}

impl NBTStorage for SpiderEntity {}

impl Mob for SpiderEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Spider.tick updates its climbing flag after movement, from the
    /// horizontal-collision result.  Doing this in `post_tick` is important:
    /// `Mob::tick` invokes it after `LivingEntity::tick`, so the generic
    /// climbing cleanup cannot erase the spider's wall-climbing state.
    fn post_tick(&self) -> EntityBaseFuture<'_, ()> {
        let entity = &self.mob_entity.living_entity.entity;
        let climbing = entity.horizontal_collision.load(Relaxed);
        let was_climbing = self
            .mob_entity
            .living_entity
            .climbing
            .swap(climbing, Relaxed);

        if climbing {
            self.mob_entity
                .living_entity
                .climbing_pos
                .store(Some(entity.block_pos.load()));
        } else if entity.on_ground.load(Relaxed) {
            self.mob_entity.living_entity.climbing_pos.store(None);
        }

        if was_climbing != climbing {
            // The generated tracker marks this field as absent for 26.x, so
            // Metadata::write safely elides it there while retaining 1.21.x
            // compatibility (where Spider flags use index 16).
            entity.send_meta_data(
                &[Metadata::new(
                    TrackedData::SPIDER_FLAGS,
                    MetaDataType::BYTE,
                    if climbing { 1u8 } else { 0u8 },
                )],
                None,
            );
        }

        Box::pin(async {})
    }
}
