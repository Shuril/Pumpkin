use std::sync::Arc;

use pumpkin_data::potion::Effect;
use pumpkin_util::Difficulty;

use crate::entity::{
    Entity, EntityBase, EntityBaseFuture, NBTStorage,
    mob::{Mob, MobEntity, spider::SpiderEntity},
};

pub struct CaveSpiderEntity {
    pub spider: Arc<SpiderEntity>,
}

impl CaveSpiderEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let spider = SpiderEntity::new(entity);
        Arc::new(Self { spider })
    }
}

impl NBTStorage for CaveSpiderEntity {}

const fn poison_duration_ticks(difficulty: Difficulty) -> i32 {
    match difficulty {
        Difficulty::Normal => 7 * 20,
        Difficulty::Hard => 15 * 20,
        Difficulty::Peaceful | Difficulty::Easy => 0,
    }
}

impl Mob for CaveSpiderEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        self.spider.get_mob_entity()
    }
    fn after_attack<'a>(
        &'a self,
        target: &'a dyn EntityBase,
        successful: bool,
    ) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move {
            if !successful {
                return;
            }
            let Some(living) = target.get_living_entity() else {
                return;
            };
            let difficulty = self.get_entity().world.load().level_info.load().difficulty;
            let duration = poison_duration_ticks(difficulty);
            if duration == 0 {
                return;
            }
            living
                .add_effect(Effect {
                    effect_type: &pumpkin_data::effect::StatusEffect::POISON,
                    duration,
                    amplifier: 0,
                    ambient: false,
                    show_particles: true,
                    show_icon: true,
                    blend: false,
                })
                .await;
        })
    }
}

#[cfg(test)]
mod tests {
    use super::poison_duration_ticks;
    use pumpkin_util::Difficulty;

    #[test]
    fn cave_spider_poison_duration_matches_vanilla_difficulty() {
        assert_eq!(poison_duration_ticks(Difficulty::Peaceful), 0);
        assert_eq!(poison_duration_ticks(Difficulty::Easy), 0);
        assert_eq!(poison_duration_ticks(Difficulty::Normal), 140);
        assert_eq!(poison_duration_ticks(Difficulty::Hard), 300);
    }
}
