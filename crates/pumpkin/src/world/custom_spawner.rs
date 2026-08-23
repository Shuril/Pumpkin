//! Vanilla-parity special spawners: `PhantomSpawner` and `PatrolSpawner`.
//!
//! Both run once per world tick, gated by the derived `spawn_enemies` flag and
//! their respective game rules, mirroring
//! `net.minecraft.world.level.levelgen.{PhantomSpawner,PatrolSpawner}`.

use std::sync::Arc;

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_rules::GameRuleRegistry;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_util::GameMode;
use pumpkin_util::difficulty::Difficulty;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkHeightmapType;
use rand::RngExt;

use crate::entity::mob::equipment::RegionalDifficulty;
use crate::entity::player::Player;
use crate::entity::r#type::from_type;
use crate::world::World;

use super::natural_spawner::{is_spawn_position_ok, is_valid_empty_spawn_block};

const PHANTOM_INSOMNIA_THRESHOLD_TICKS: i32 = 72000;
const PATROL_VILLAGE_PROXIMITY_BLOCKS: f64 = 32.0;
const MUSHROOM_FIELDS_REGISTRY_ID: &str = "mushroom_fields";

#[derive(Debug)]
pub struct CustomSpawners {
    phantom_next_tick: i32,
    patrol_next_tick: i32,
}

impl Default for CustomSpawners {
    fn default() -> Self {
        Self::new()
    }
}

impl CustomSpawners {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            phantom_next_tick: 0,
            patrol_next_tick: 0,
        }
    }

    pub async fn tick(&mut self, world: &Arc<World>, spawn_enemies: bool) {
        if !spawn_enemies {
            return;
        }
        let level_info = world.level_info.load();
        self.tick_phantoms(world, &level_info.game_rules).await;
        self.tick_patrols(world, &level_info.game_rules).await;
    }

    async fn tick_phantoms(&mut self, world: &Arc<World>, game_rules: &GameRuleRegistry) {
        if !game_rules.spawn_phantoms {
            return;
        }
        self.phantom_next_tick -= 1;
        if self.phantom_next_tick > 0 {
            return;
        }
        self.phantom_next_tick += (60 + rand::rng().random_range(0..60)) * 20;

        let has_skylight = world.dimension.has_skylight;
        if sky_darken(world) < 5 && has_skylight {
            return;
        }
        for player in world.players.load().iter() {
            if player.gamemode.load() == GameMode::Spectator {
                continue;
            }
            let player_pos = player.living_entity.entity.block_pos.load();
            if has_skylight && (player_pos.0.y < world.sea_level || !world.can_see_sky(&player_pos))
            {
                continue;
            }
            let difficulty = RegionalDifficulty::at(world, player_pos.0.to_f64());
            if difficulty.effective_difficulty <= rand::rng().random::<f32>() * 3.0 {
                continue;
            }
            let insomnia = insomnia_ticks(player).await;
            if rand::rng().random_range(0..insomnia) < PHANTOM_INSOMNIA_THRESHOLD_TICKS {
                continue;
            }
            let (spawn_pos, group_size) = {
                let mut rng = rand::rng();
                (
                    BlockPos(Vector3::new(
                        player_pos.0.x + rng.random_range(-10..=10),
                        player_pos.0.y + 20 + rng.random_range(0..15),
                        player_pos.0.z + rng.random_range(-10..=10),
                    )),
                    1 + rng.random_range(0..=(difficulty_id(world) as usize)),
                )
            };
            if !is_valid_empty_spawn_block(world.get_block_state(&spawn_pos)) {
                continue;
            }
            for _ in 0..group_size {
                spawn_flying(world, spawn_pos).await;
            }
        }
    }

    async fn tick_patrols(&mut self, world: &Arc<World>, game_rules: &GameRuleRegistry) {
        if !game_rules.spawn_patrols {
            return;
        }
        self.patrol_next_tick -= 1;
        if self.patrol_next_tick > 0 {
            return;
        }
        self.patrol_next_tick += 12000 + rand::rng().random_range(0..1200);

        if sky_darken(world) < 4 || rand::rng().random_range(0..5) != 0 {
            return;
        }
        let players = world.players.load();
        if players.is_empty() {
            return;
        }
        let player_index = rand::rng().random_range(0..players.len());
        let player = &players[player_index];
        if player.gamemode.load() == GameMode::Spectator {
            return;
        }
        let player_pos = player.living_entity.entity.block_pos.load();
        let poi = world.villager_poi.lock().await;
        if poi.has_job_site_within(&player_pos, PATROL_VILLAGE_PROXIMITY_BLOCKS) {
            return;
        }
        drop(poi);

        let (mut pos_x, mut pos_z) = {
            let mut rng = rand::rng();
            let sign_x = if rng.random_bool(0.5) { 1 } else { -1 };
            let sign_z = if rng.random_bool(0.5) { 1 } else { -1 };
            (
                player_pos.0.x + (24 + rng.random_range(0..24)) * sign_x,
                player_pos.0.z + (24 + rng.random_range(0..24)) * sign_z,
            )
        };

        if !has_chunks_loaded(world, pos_x, pos_z, 10) {
            return;
        }
        let biome_probe = BlockPos(Vector3::new(pos_x, player_pos.0.y, pos_z));
        if world.get_biome(&biome_probe).registry_id == MUSHROOM_FIELDS_REGISTRY_ID {
            return;
        }

        let group_size = RegionalDifficulty::at(world, biome_probe.0.to_f64())
            .effective_difficulty
            .ceil() as usize
            + 1;
        for index in 0..group_size {
            let pos_y = surface_y(world, pos_x, pos_z);
            let spawned = spawn_patrol_member(
                world,
                BlockPos(Vector3::new(pos_x, pos_y, pos_z)),
                index == 0,
            )
            .await;
            if index == 0 && !spawned {
                return;
            }
            let jitter_x = {
                let mut rng = rand::rng();
                rng.random_range(0..5) - rng.random_range(0..5)
            };
            let jitter_z = {
                let mut rng = rand::rng();
                rng.random_range(0..5) - rng.random_range(0..5)
            };
            pos_x += jitter_x;
            pos_z += jitter_z;
        }
    }
}

fn difficulty_id(world: &World) -> u8 {
    match world.level_info.load().difficulty {
        Difficulty::Peaceful => 0,
        Difficulty::Easy => 1,
        Difficulty::Normal => 2,
        Difficulty::Hard => 3,
    }
}

async fn insomnia_ticks(player: &Player) -> i32 {
    player
        .stats
        .lock()
        .await
        .get(
            crate::entity::player::statistics::StatisticCategory::Custom,
            crate::entity::player::statistics::CustomStatistic::TimeSinceRest as i32,
        )
        .max(1)
}

async fn spawn_flying(world: &Arc<World>, pos: BlockPos) {
    let position = Vector3::new(
        f64::from(pos.0.x) + 0.5,
        f64::from(pos.0.y),
        f64::from(pos.0.z) + 0.5,
    );
    let entity = from_type(&EntityType::PHANTOM, position, world, uuid::Uuid::new_v4());
    world.spawn_entity(entity).await;
}

async fn spawn_patrol_member(world: &Arc<World>, pos: BlockPos, leader: bool) -> bool {
    if world
        .get_block_light_level(&pos)
        .is_some_and(|light| light > 8)
    {
        return false;
    }
    if !is_spawn_position_ok(world, &pos, &EntityType::PILLAGER) {
        return false;
    }
    let position = Vector3::new(
        f64::from(pos.0.x) + 0.5,
        f64::from(pos.0.y),
        f64::from(pos.0.z) + 0.5,
    );
    let entity = from_type(&EntityType::PILLAGER, position, world, uuid::Uuid::new_v4());
    world.spawn_entity(entity.clone()).await;

    if leader && let Some(living) = entity.get_living_entity() {
        living
            .entity_equipment
            .lock()
            .await
            .put(&EquipmentSlot::HEAD, ItemStack::new(1, &Item::WHITE_BANNER));
        living.send_equipment_changes(&[(
            EquipmentSlot::HEAD,
            ItemStack::new(1, &Item::WHITE_BANNER),
        )]);
    }
    true
}

fn surface_y(world: &World, x: i32, z: i32) -> i32 {
    world.get_heightmap_height(ChunkHeightmapType::MotionBlocking, x, z) + 1
}

fn has_chunks_loaded(world: &World, x: i32, z: i32, radius: i32) -> bool {
    for chunk_x in ((x - radius) >> 4)..=((x + radius) >> 4) {
        for chunk_z in ((z - radius) >> 4)..=((z + radius) >> 4) {
            if !world
                .level
                .loaded_chunks
                .contains_key(&Vector2::new(chunk_x, chunk_z))
            {
                return false;
            }
        }
    }
    true
}

/// Port of the overworld clock's `SKY_LIGHT_LEVEL` timeline (`Timelines.OVERWORLD_DAY`)
/// combined with `Level#getSkyDarken`: `15 - sky_light_level`.
fn sky_darken(world: &World) -> i32 {
    let time_of_day = world
        .level_time
        .try_lock()
        .map_or(0, |time| time.time_of_day);
    sky_darken_at(time_of_day)
}

const SKY_LIGHT_KEYFRAMES: [(i64, f32); 4] = [
    (133, 1.0),
    (11_867, 1.0),
    (13_670, 4.0 / 15.0),
    (22_330, 4.0 / 15.0),
];

fn sky_darken_at(time_of_day: i64) -> i32 {
    let tick = time_of_day.rem_euclid(24_000);
    let multiplier = sky_light_multiplier(tick);
    (15.0 - 15.0 * multiplier) as i32
}

fn sky_light_multiplier(tick: i64) -> f32 {
    let (first_tick, first_value) = SKY_LIGHT_KEYFRAMES[0];
    let (last_tick, last_value) = SKY_LIGHT_KEYFRAMES[SKY_LIGHT_KEYFRAMES.len() - 1];

    if tick < first_tick {
        let span = (24_000 - last_tick) + first_tick;
        let progress = (tick + 24_000 - last_tick) as f32 / span as f32;
        return last_value + (first_value - last_value) * progress;
    }

    for window in SKY_LIGHT_KEYFRAMES.windows(2) {
        let (start_tick, start_value) = window[0];
        let (end_tick, end_value) = window[1];
        if tick < end_tick {
            let progress = (tick - start_tick) as f32 / (end_tick - start_tick) as f32;
            return start_value + (end_value - start_value) * progress;
        }
    }
    last_value
}

#[cfg(test)]
mod tests {
    use super::{PHANTOM_INSOMNIA_THRESHOLD_TICKS, sky_darken_at};

    #[test]
    fn sky_darken_peaks_at_midnight_and_is_zero_during_day() {
        assert_eq!(sky_darken_at(6_000), 0);
        assert_eq!(sky_darken_at(10_000), 0);
        assert_eq!(sky_darken_at(18_000), 11);
        assert_eq!(sky_darken_at(20_000), 11);
        let dusk = sky_darken_at(13_000);
        assert!(dusk > 0 && dusk < 11);
    }

    #[test]
    fn phantom_threshold_matches_three_in_game_days() {
        assert_eq!(PHANTOM_INSOMNIA_THRESHOLD_TICKS, 72_000);
    }
}
