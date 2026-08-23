use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::GameMode;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::text::TextComponent;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Weak};
use uuid::Uuid;

use crate::entity::{
    Entity, EntityBase, EntityBaseFuture,
    ai::goal::{look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal},
    mob::{Mob, MobEntity},
    player::Player,
    projectile::wither_skull::WitherSkullEntity,
};
use crate::world::bossbar::{Bossbar, BossbarColor, BossbarDivisions, BossbarFlags};

const TARGET_RANGE: f64 = 48.0;
const HOVER_HEIGHT: f64 = 6.0;
const HOVER_HORIZONTAL_SPEED: f64 = 0.45;
const SHOOT_INTERVAL_TICKS: i32 = 60;
const BOSSBAR_SYNC_TICKS: i32 = 10;
const SKULL_SPEED: f64 = 1.2;

pub struct WitherEntity {
    pub mob_entity: MobEntity,
    bossbar_uuid: Uuid,
    bossbar_players: tokio::sync::Mutex<Vec<Uuid>>,
    shoot_cooldown: AtomicI32,
}

impl WitherEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let wither = Self {
            mob_entity,
            bossbar_uuid: Uuid::new_v4(),
            bossbar_players: tokio::sync::Mutex::new(Vec::new()),
            shoot_cooldown: AtomicI32::new(SHOOT_INTERVAL_TICKS),
        };
        let mob_arc = Arc::new(wither);
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

            goal_selector.add_goal(
                8,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(8, Box::new(RandomLookAroundGoal::default()));
        };

        mob_arc
    }

    async fn closest_player(&self) -> Option<Arc<Player>> {
        let world = self.mob_entity.living_entity.entity.world.load();
        let pos = self.mob_entity.living_entity.entity.pos.load();
        let player = world.get_closest_player(pos, TARGET_RANGE)?;
        (player.gamemode.load() != GameMode::Spectator).then_some(player)
    }

    fn make_bossbar(&self) -> Bossbar {
        Bossbar {
            uuid: self.bossbar_uuid,
            title: TextComponent::translate_cross(
                "entity.minecraft.wither",
                "entity.minecraft.wither",
                [],
            ),
            health: 1.0,
            color: BossbarColor::Purple,
            division: BossbarDivisions::NoDivision,
            flags: BossbarFlags::empty(),
        }
    }
}

impl Mob for WitherEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn get_mob_gravity(&self) -> f64 {
        0.0
    }

    fn mob_tick<'a>(&'a self, _caller: &'a Arc<dyn EntityBase>) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move {
            let world = self.mob_entity.living_entity.entity.world.load();
            let pos = self.mob_entity.living_entity.entity.pos.load();

            // Boss bar audience + health sync
            if world.level_time.try_lock().map_or(0, |t| t.world_age)
                % i64::from(BOSSBAR_SYNC_TICKS)
                == 0
            {
                let bar = self.make_bossbar();
                let mut shown = self.bossbar_players.lock().await;
                for player in world.players.load().iter() {
                    let in_range = player
                        .living_entity
                        .entity
                        .pos
                        .load()
                        .squared_distance_to_vec(&pos)
                        <= TARGET_RANGE * TARGET_RANGE;
                    let known = shown.contains(&player.gameprofile.id);
                    match (in_range, known) {
                        (true, false) => {
                            player.send_bossbar(&bar).await;
                            shown.push(player.gameprofile.id);
                        }
                        (false, true) => {
                            player.remove_bossbar(self.bossbar_uuid).await;
                        }
                        _ => {}
                    }
                }
                let health_fraction =
                    (self.mob_entity.living_entity.health.load() / 300.0).clamp(0.0, 1.0);
                for uuid in shown.iter() {
                    if let Some(player) = world
                        .players
                        .load()
                        .iter()
                        .find(|p| &p.gameprofile.id == uuid)
                    {
                        player
                            .update_bossbar_health(&self.bossbar_uuid, health_fraction)
                            .await;
                    }
                }
            }

            let Some(target) = self.closest_player().await else {
                return;
            };
            let target_pos = target.get_entity().pos.load();

            // Hover above and drift toward the player.
            let hover_point = Vector3::new(target_pos.x, target_pos.y + HOVER_HEIGHT, target_pos.z);
            let delta = hover_point.sub(&pos);
            let horizontal_len = delta.x.hypot(delta.z).max(0.001);
            let entity = &self.mob_entity.living_entity.entity;
            entity.set_velocity(Vector3::new(
                delta.x / horizontal_len * HOVER_HORIZONTAL_SPEED,
                (delta.y * 0.08).clamp(-0.25, 0.25),
                delta.z / horizontal_len * HOVER_HORIZONTAL_SPEED,
            ));
            entity.set_rotation(f64::atan2(-delta.x, delta.z) as f32, 0.0);

            // Fire a wither skull.
            let cooldown = self.shoot_cooldown.fetch_sub(1, Ordering::Relaxed);
            if cooldown <= 0 {
                self.shoot_cooldown
                    .store(SHOOT_INTERVAL_TICKS, Ordering::Relaxed);

                let skull_pos = pos.add(&Vector3::new(0.0, 2.9, 0.0));
                let aim = target_pos.add(&Vector3::new(0.0, 1.0, 0.0)).sub(&skull_pos);
                let len = aim.length().max(0.001);
                let velocity = Vector3::new(
                    aim.x / len * SKULL_SPEED,
                    aim.y / len * SKULL_SPEED,
                    aim.z / len * SKULL_SPEED,
                );

                let skull_entity = Entity::new(world.clone(), skull_pos, &EntityType::WITHER_SKULL);
                let mut skull = WitherSkullEntity::new_shot(skull_entity, entity);
                skull.thrown.entity.set_velocity(velocity);
                world
                    .spawn_entity(Arc::new(skull) as Arc<dyn crate::entity::EntityBase>)
                    .await;
                world.play_sound(Sound::EntityWitherShoot, SoundCategory::Hostile, &pos);
            }
        })
    }
}
