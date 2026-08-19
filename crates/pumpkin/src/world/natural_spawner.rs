use crate::entity::EntityBase;
use crate::entity::r#type::{check_spawn_rules, from_type};
use crate::world::World;
use arc_swap::ArcSwap;
use pumpkin_data::biome::Spawner;
use pumpkin_data::chunk::Biome;
use pumpkin_data::entity::{EntityType, MobCategory, SpawnLocation};
use pumpkin_data::tag::Fluid::{MINECRAFT_LAVA, MINECRAFT_WATER};
use pumpkin_data::tag::Taggable;
use pumpkin_data::tag::WorldgenBiome::MINECRAFT_REDUCE_WATER_AMBIENT_SPAWNS;
use pumpkin_data::tag::{self, Block::MINECRAFT_PREVENT_MOB_SPAWNING_INSIDE};
use pumpkin_data::{Block, BlockDirection, BlockState};
use pumpkin_util::GameMode;
use pumpkin_util::math::boundingbox::{BoundingBox, EntityDimensions};
use pumpkin_util::math::get_section_cord;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::{RandomGenerator, RandomImpl};
use pumpkin_world::chunk::{ChunkData, ChunkHeightmapType};
use pumpkin_world::generation::proto_chunk::GenerationCache;
use pumpkin_world::generation::structure::structures::create_chunk_random;
use std::fmt;
use std::sync::Arc;
use uuid::Uuid;

const MAGIC_NUMBER: i32 = 17 * 17;

use dashmap::DashMap;
use std::sync::atomic::{AtomicI32, Ordering::Relaxed};

pub struct MobCounts([AtomicI32; 8]);

impl Default for MobCounts {
    fn default() -> Self {
        Self(std::array::from_fn(|_| AtomicI32::new(0)))
    }
}

impl fmt::Debug for MobCounts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|a| a.load(Relaxed)))
            .finish()
    }
}

impl Clone for MobCounts {
    fn clone(&self) -> Self {
        Self(std::array::from_fn(|i| {
            AtomicI32::new(self.0[i].load(Relaxed))
        }))
    }
}

impl MobCounts {
    #[inline]
    pub fn add(&self, category: &'static MobCategory) {
        self.0[category.id].fetch_add(1, Relaxed);
    }

    #[inline]
    pub fn remove(&self, category: &'static MobCategory) {
        let _ = self.0[category.id]
            .fetch_update(Relaxed, Relaxed, |value| Some(value.saturating_sub(1)));
    }
    #[inline]
    pub fn can_spawn(&self, category: &'static MobCategory) -> bool {
        self.0[category.id].load(Relaxed) < category.max
    }
}

pub struct LocalMobCapCalculator {
    player_mob_counts: DashMap<i32, MobCounts>,
    players_near_chunk: DashMap<Vector2<i32>, Vec<i32>>,
}

impl Clone for LocalMobCapCalculator {
    fn clone(&self) -> Self {
        let player_mob_counts = DashMap::new();
        for r in &self.player_mob_counts {
            player_mob_counts.insert(*r.key(), r.value().clone());
        }
        let players_near_chunk = DashMap::new();
        for r in &self.players_near_chunk {
            players_near_chunk.insert(*r.key(), r.value().clone());
        }
        Self {
            player_mob_counts,
            players_near_chunk,
        }
    }
}

impl Default for LocalMobCapCalculator {
    fn default() -> Self {
        Self {
            player_mob_counts: DashMap::new(),
            players_near_chunk: DashMap::new(),
        }
    }
}

impl fmt::Debug for LocalMobCapCalculator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("LocalMobCapCalculator")
            .field("world", &"<skipped>")
            .finish()
    }
}

impl LocalMobCapCalculator {
    const fn calc_distance(chunk_pos: Vector2<i32>, player_pos: &Vector3<f64>) -> f64 {
        let dx = ((chunk_pos.x << 4) + 8) as f64 - player_pos.x;
        let dy = ((chunk_pos.y << 4) + 8) as f64 - player_pos.z;
        dx * dx + dy * dy
    }

    fn get_players_near(&self, world: &World, chunk_pos: Vector2<i32>) -> Vec<i32> {
        // Player positions are mutable every tick.  Vanilla's
        // LocalMobCapCalculator is refreshed from the current chunk/player
        // map; retaining the first result forever would leave a moved player
        // contributing caps to the old chunk and never to the new one.
        let mut players = Vec::new();
        for player in world.players.load().iter() {
            if player.gamemode.load() == GameMode::Spectator {
                continue;
            }
            if Self::calc_distance(chunk_pos, &player.position()) < 16384. {
                players.push(player.entity_id());
            }
        }
        self.players_near_chunk.insert(chunk_pos, players.clone());
        players
    }

    pub fn add_mob(&self, chunk_pos: Vector2<i32>, world: &World, category: &'static MobCategory) {
        let players = self.get_players_near(world, chunk_pos);
        for player in players {
            self.player_mob_counts
                .entry(player)
                .or_default()
                .add(category);
        }
    }

    pub fn remove_mob(
        &self,
        chunk_pos: Vector2<i32>,
        world: &World,
        category: &'static MobCategory,
    ) {
        let players = self.get_players_near(world, chunk_pos);
        for player in players {
            if let Some(count) = self.player_mob_counts.get(&player) {
                count.remove(category);
            }
        }
    }

    pub fn can_spawn(
        &self,
        category: &'static MobCategory,
        world: &World,
        chunk_pos: Vector2<i32>,
    ) -> bool {
        let players = self.get_players_near(world, chunk_pos);
        for player in players {
            if let Some(count) = self.player_mob_counts.get(&player) {
                if count.can_spawn(category) {
                    return true;
                }
            } else {
                return true;
            }
        }
        false
    }
}

#[derive(Debug, Clone)]
struct PointCharge(Vector3<f64>, f64);

impl PointCharge {
    fn get_potential_change(&self, pos: &BlockPos) -> f64 {
        let dst = self.0.sub(&pos.to_f64()).length();
        // NaturalSpawner's PotentialCalculator treats a charge at the exact
        // candidate position as zero potential.  Returning 0 here avoids an
        // infinity/NaN that would otherwise permanently reject every spawn in
        // that block and is also safe for malformed duplicate charge data.
        if dst <= f64::EPSILON {
            0.0
        } else {
            self.1 / dst
        }
    }
}

#[derive(Default, Debug)]
struct PotentialCalculator(std::sync::Mutex<Vec<PointCharge>>);

impl Clone for PotentialCalculator {
    fn clone(&self) -> Self {
        Self(std::sync::Mutex::new(
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        ))
    }
}

impl PotentialCalculator {
    pub fn add_charge(&self, pos: &BlockPos, charge: f64) {
        if charge != 0. {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(PointCharge(pos.to_f64(), charge));
        }
    }

    pub fn remove_charge(&self, pos: &BlockPos, charge: f64) {
        if charge != 0. {
            let mut charges = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let pos_f64 = pos.to_f64();
            if let Some(idx) = charges.iter().position(|c| c.0 == pos_f64 && c.1 == charge) {
                charges.swap_remove(idx);
            }
        }
    }
    pub fn get_potential_energy_change(&self, pos: &BlockPos, charge: f64) -> f64 {
        if charge == 0. {
            return 0.;
        }
        let mut sum: f64 = 0.;
        let charges = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for i in charges.iter() {
            sum += i.get_potential_change(pos);
        }
        sum * charge
    }
}

use crossbeam::atomic::AtomicCell;

pub struct SpawnState {
    spawnable_chunk_count: i32,
    pub mob_category_counts: MobCounts,
    spawn_potential: PotentialCalculator,
    local_mob_cap_calculator: LocalMobCapCalculator,
    // unmodifiable_mob_category_counts: MobCounts, seems only for debug
    last_checked: AtomicCell<Option<(BlockPos, &'static EntityType, f64)>>,
}

impl Clone for SpawnState {
    fn clone(&self) -> Self {
        Self {
            spawnable_chunk_count: self.spawnable_chunk_count,
            mob_category_counts: self.mob_category_counts.clone(),
            spawn_potential: self.spawn_potential.clone(),
            local_mob_cap_calculator: self.local_mob_cap_calculator.clone(),
            last_checked: AtomicCell::new(self.last_checked.load()),
        }
    }
}

impl fmt::Debug for SpawnState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("SpawnState")
            .field("spawnable_chunk_count", &self.spawnable_chunk_count)
            .field("mob_category_counts", &self.mob_category_counts)
            .field("spawn_potential", &self.spawn_potential)
            .field("local_mob_cap_calculator", &self.local_mob_cap_calculator)
            .field("last_checked", &self.last_checked)
            .finish()
    }
}

impl SpawnState {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            spawnable_chunk_count: 0,
            mob_category_counts: MobCounts::default(),
            spawn_potential: PotentialCalculator::default(),
            local_mob_cap_calculator: LocalMobCapCalculator::default(),
            last_checked: AtomicCell::new(None),
        }
    }

    pub const fn set_spawnable_chunk_count(&mut self, count: i32) {
        self.spawnable_chunk_count = count;
    }

    pub fn add_entity(&self, world: &World, entity: &dyn EntityBase) {
        if !entity.should_save() {
            return;
        }
        let base_entity = entity.get_entity();
        let entity_type = base_entity.entity_type;
        if !entity_type.mob || entity_type.category == &MobCategory::MISC {
            return;
        }
        if base_entity.persistence_required.load(Relaxed) {
            return;
        }
        let entity_pos = base_entity.block_pos.load();
        let biome = world.level.get_rough_biome(&entity_pos);
        if let Some(cost) = biome.spawn_costs.get(entity_type.resource_name) {
            self.spawn_potential.add_charge(&entity_pos, cost.charge);
        }
        if entity_type.mob {
            self.local_mob_cap_calculator.add_mob(
                base_entity.chunk_pos.load(),
                world,
                entity_type.category,
            );
            self.mob_category_counts.add(entity_type.category);
        }
    }

    pub fn remove_entity(&self, world: &World, entity: &dyn EntityBase) {
        if !entity.should_save() {
            return;
        }
        let base_entity = entity.get_entity();
        let entity_type = base_entity.entity_type;
        if !entity_type.mob || entity_type.category == &MobCategory::MISC {
            return;
        }
        if base_entity.persistence_required.load(Relaxed) {
            return;
        }
        let entity_pos = base_entity.block_pos.load();
        let biome = world.level.get_rough_biome(&entity_pos);
        if let Some(cost) = biome.spawn_costs.get(entity_type.resource_name) {
            self.spawn_potential.remove_charge(&entity_pos, cost.charge);
        }
        if entity_type.mob {
            self.local_mob_cap_calculator.remove_mob(
                base_entity.chunk_pos.load(),
                world,
                entity_type.category,
            );
            self.mob_category_counts.remove(entity_type.category);
        }
    }

    pub fn new(
        chunk_count: i32,
        entities: &ArcSwap<Vec<Arc<dyn EntityBase>>>,
        world: &Arc<World>,
    ) -> Self {
        let potential = PotentialCalculator::default();
        let local_mob_cap = LocalMobCapCalculator::default();
        let counter = MobCounts::default();
        let active_chunks = world.active_chunks.load();
        for entity in entities.load().iter() {
            let entity = entity.get_entity();
            let entity_type = entity.entity_type;
            if !entity_type.mob || entity_type.category == &MobCategory::MISC {
                continue;
            }
            // Vanilla's NaturalSpawner excludes mobs marked
            // `PersistenceRequired` from natural-spawn caps.  Name tags and
            // other interactions set this bit; counting those mobs would
            // incorrectly suppress unrelated natural spawns.
            if entity.persistence_required.load(Relaxed) {
                continue;
            }
            let chunk_pos = entity.chunk_pos.load();
            if !active_chunks.contains(&chunk_pos) {
                continue;
            }
            let entity_pos = entity.block_pos.load();
            let biome = world.level.get_rough_biome(&entity_pos);
            if let Some(cost) = biome.spawn_costs.get(entity_type.resource_name) {
                potential.add_charge(&entity_pos, cost.charge);
            }
            if entity_type.mob {
                local_mob_cap.add_mob(chunk_pos, world, entity_type.category);
            }
            counter.add(entity_type.category);
        }
        Self {
            spawnable_chunk_count: chunk_count,
            mob_category_counts: counter,
            spawn_potential: potential,
            local_mob_cap_calculator: local_mob_cap,
            last_checked: AtomicCell::new(None),
        }
    }
    #[inline]
    pub fn can_spawn_for_category_global(&self, category: &'static MobCategory) -> bool {
        self.mob_category_counts.0[category.id].load(Relaxed)
            < category.max * self.spawnable_chunk_count / MAGIC_NUMBER
    }
    pub fn can_spawn_for_category_local(
        &self,
        world: &Arc<World>,
        category: &'static MobCategory,
        chunk_pos: Vector2<i32>,
    ) -> bool {
        self.local_mob_cap_calculator
            .can_spawn(category, world, chunk_pos)
    }
    pub fn can_spawn(
        &self,
        entity_type: &'static EntityType,
        pos: &BlockPos,
        world: &Arc<World>,
    ) -> bool {
        // TODO get biome
        let biome = world.level.get_rough_biome(pos);
        biome
            .spawn_costs
            .get(entity_type.resource_name)
            .map_or_else(
                || {
                    self.last_checked.store(Some((*pos, entity_type, 0.)));
                    true
                },
                |cost| {
                    self.last_checked
                        .store(Some((*pos, entity_type, cost.charge)));
                    self.spawn_potential
                        .get_potential_energy_change(pos, cost.charge)
                        <= cost.energy_budget
                },
            )
    }
    pub fn after_spawn(
        &self,
        entity_type: &'static EntityType,
        pos: &BlockPos,
        world: &Arc<World>,
    ) {
        let charge = if let Some((l_pos, l_type, l_charge)) = self.last_checked.load()
            && l_pos.eq(pos)
            && l_type == entity_type
        {
            Some(l_charge)
        } else {
            None
        };

        let charge = charge.unwrap_or_else(|| {
            // TODO get biome
            let biome = world.level.get_rough_biome(pos);
            biome
                .spawn_costs
                .get(entity_type.resource_name)
                .map_or(0., |cost| cost.charge)
        });

        self.spawn_potential.add_charge(pos, charge);
        self.mob_category_counts.add(entity_type.category);
        self.local_mob_cap_calculator.add_mob(
            Vector2::<i32>::new(get_section_cord(pos.0.x), get_section_cord(pos.0.z)),
            world,
            entity_type.category,
        );
    }
}

#[must_use]
pub fn get_filtered_spawning_categories(
    state: &SpawnState,
    spawn_friendlies: bool,
    spawn_enemies: bool,
    spawn_passives: bool,
) -> Vec<&'static MobCategory> {
    let mut ret = Vec::with_capacity(MobCategory::SPAWNING_CATEGORIES.len());
    for category in MobCategory::SPAWNING_CATEGORIES {
        let is_type_allowed = if category.is_friendly {
            spawn_friendlies
        } else {
            spawn_enemies
        };

        if !is_type_allowed {
            continue;
        }

        if category.is_persistent && !spawn_passives {
            continue;
        }

        if state.can_spawn_for_category_global(category) {
            ret.push(category);
        }
    }
    ret
}

pub fn spawn_for_chunk(
    world: &Arc<World>,
    chunk_pos: Vector2<i32>,
    chunk: &Arc<ChunkData>,
    spawn_state: &SpawnState,
    spawn_list: &Vec<&'static MobCategory>,
    is_thundering: bool,
    world_age: i64,
) -> Vec<Arc<dyn EntityBase>> {
    // debug!("spawn for chunk {:?}", chunk_pos);
    let mut entities = Vec::new();
    // ServerLevel owns one random source, but Pumpkin processes chunks in a
    // JoinSet.  A shared/thread-local RNG would therefore make spawn results
    // depend on task scheduling.  Derive one stream per chunk and tick so the
    // complete candidate order is stable even when chunks run concurrently.
    let mut random = spawn_random(world.level.seed.0, world_age, chunk_pos);
    for category in spawn_list {
        if spawn_state.can_spawn_for_category_local(world, category, chunk_pos) {
            let random_pos =
                get_random_pos_within_with_random(world.min_y, &chunk_pos, chunk, &mut random);
            if random_pos.0.y > world.min_y {
                entities.extend(spawn_category_for_position(
                    category,
                    world,
                    random_pos,
                    &chunk_pos,
                    spawn_state,
                    is_thundering,
                    &mut random,
                ));
            }
        }
    }
    entities
}
pub fn get_random_pos_within(
    min_y: i32,
    chunk_pos: &Vector2<i32>,
    chunk: &Arc<ChunkData>,
) -> BlockPos {
    // Kept as a stable helper for callers outside the regular server tick.
    // The active natural-spawn path uses `get_random_pos_within_with_random` below,
    // because it must share the per-chunk tick stream with group spawning.
    let mut random =
        RandomGenerator::Legacy(pumpkin_util::random::legacy_rand::LegacyRand::from_seed(
            ((i64::from(chunk_pos.x) << 32) ^ i64::from(chunk_pos.y)) as u64,
        ));
    get_random_pos_within_with_random(min_y, chunk_pos, chunk, &mut random)
}

fn get_random_pos_within_with_random(
    min_y: i32,
    chunk_pos: &Vector2<i32>,
    chunk: &Arc<ChunkData>,
    random: &mut RandomGenerator,
) -> BlockPos {
    let x = (chunk_pos.x << 4) + random.next_bounded_i32(16);
    let z = (chunk_pos.y << 4) + random.next_bounded_i32(16);
    let temp_y = chunk
        .heightmap
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(ChunkHeightmapType::WorldSurface, x, z, chunk.section.min_y)
        + 1;
    let y = random.next_inbetween_i32(min_y, temp_y);
    BlockPos::new(x, y, z)
}

pub fn spawn_mobs_for_chunk_generation(
    world: &Arc<World>,
    cache: &mut dyn GenerationCache,
    biome: &'static Biome,
    chunk_x: i32,
    chunk_z: i32,
) {
    // Vanilla's chunk-generation path is still gated by the global mob
    // spawning gamerule.  Without this guard, newly generated chunks could
    // silently populate passive mobs even after `/gamerule doMobSpawning
    // false`, while the normal tick path correctly remained empty.
    if !world.level_info.load().game_rules.spawn_mobs {
        return;
    }

    let mob_settings = &biome.spawners;
    let creatures = &mob_settings.creature;

    if creatures.is_empty() {
        return;
    }

    let xo = chunk_x << 4;
    let zo = chunk_z << 4;

    // Chunk-generation spawning must be reproducible from the world seed.  The
    // old implementation used the process-global `rand` generator here, which
    // made the contents of a newly generated chunk depend on thread scheduling
    // and could produce different mobs after a restart.  Vanilla derives one
    // population RNG from the world seed and chunk coordinates; using the same
    // chunk-local stream also keeps all random choices in this loop stable.
    let mut random = chunk_generation_random(world.level.seed.0, chunk_x, chunk_z);

    while random.next_f32() < biome.creature_spawn_probability {
        let spawner_data = &creatures[random.next_bounded_i32(creatures.len() as i32) as usize];

        let count = spawner_data.min_count
            + random.next_bounded_i32((1 + spawner_data.max_count - spawner_data.min_count).max(1));
        let name = spawner_data
            .r#type
            .strip_prefix("minecraft:")
            .unwrap_or(spawner_data.r#type);
        let Some(entity_type) = EntityType::from_name(name) else {
            return;
        };

        let mut x = xo + random.next_bounded_i32(16);
        let mut z = zo + random.next_bounded_i32(16);
        let start_x = x;
        let start_z = z;

        for _ in 0..count {
            let mut success = false;

            // Try 4 times to find a valid spot in the immediate area
            for _ in 0..4 {
                if success {
                    break;
                }

                let pos = get_top_non_colliding_pos(world, cache, entity_type, x, z);

                // Chunk-population spawning used to stop after the cache
                // support check. That allowed generated mobs to bypass the
                // same difficulty/light/biome predicates used by the live
                // natural-spawn path, and it could place a mob into a
                // collision volume when a heightmap was stale. Keep all
                // decisions on this chunk-local RNG stream so generation is
                // reproducible across worker scheduling.
                let dimensions = natural_spawn_dimensions(entity_type);
                let spawn_box = BoundingBox::new_from_pos(
                    f64::from(pos.0.x) + 0.5,
                    f64::from(pos.0.y),
                    f64::from(pos.0.z) + 0.5,
                    &dimensions,
                );
                if is_spawn_position_ok_cache(cache, &pos, entity_type)
                    && is_within_world_border_sync(world, &pos)
                    && entity_type.summonable
                    && check_spawn_rules(entity_type, world, &pos, false, &mut random)
                    && world.is_space_empty(spawn_box)
                {
                    let spawn_pos_f64 = Vector3::new(
                        f64::from(pos.0.x) + 0.5,
                        f64::from(pos.0.y),
                        f64::from(pos.0.z) + 0.5,
                    );

                    let entity = from_type(entity_type, spawn_pos_f64, world, Uuid::new_v4());
                    entity
                        .get_entity()
                        .set_rotation(random.next_f32() * 360., 0.);
                    world.spawn_entity_non_save(&entity);
                    success = true;
                }

                // Random jitter for the next mob in the group
                x += random.next_bounded_i32(5) - random.next_bounded_i32(5);
                z += random.next_bounded_i32(5) - random.next_bounded_i32(5);

                // Keep group within the chunk bounds
                if x < xo || x >= xo + 16 || z < zo || z >= zo + 16 {
                    x = start_x;
                    z = start_z;
                }
            }
        }
    }
}

/// Returns the deterministic population stream used by chunk-generation mob
/// spawning.  Keeping this derivation in one function makes it impossible for
/// callers to accidentally fall back to a process-global RNG and gives the
/// seed contract a small, isolated unit-test surface.
fn chunk_generation_random(
    world_seed: u64,
    chunk_x: i32,
    chunk_z: i32,
) -> pumpkin_util::random::RandomGenerator {
    create_chunk_random(world_seed as i64, chunk_x, chunk_z)
}

/// Returns the random stream used by one natural-spawn pass.  The age salt is
/// deliberately mixed before `create_chunk_random`: a chunk gets a fresh
/// vanilla-style stream every server tick, while two chunks never consume the
/// same shared state.  Wrapping arithmetic is intentional for the signed
/// Minecraft seed domain.
pub(crate) fn spawn_random(
    world_seed: u64,
    world_age: i64,
    chunk_pos: Vector2<i32>,
) -> RandomGenerator {
    let tick_seed = world_seed ^ (world_age as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    create_chunk_random(tick_seed as i64, chunk_pos.x, chunk_pos.y)
}

#[cfg(test)]
mod generation_random_tests {
    use super::{chunk_generation_random, spawn_random};
    use pumpkin_util::math::vector2::Vector2;
    use pumpkin_util::random::RandomImpl;

    #[test]
    fn chunk_generation_population_random_is_reproducible_and_chunk_local() {
        let mut first = chunk_generation_random(0x5eed, 12, -4);
        let mut second = chunk_generation_random(0x5eed, 12, -4);
        assert_eq!(first.next_i64(), second.next_i64());
        assert_eq!(first.next_i64(), second.next_i64());

        let mut other_chunk = chunk_generation_random(0x5eed, 13, -4);
        assert_ne!(first.next_i64(), other_chunk.next_i64());
    }

    #[test]
    fn natural_spawn_stream_is_reproducible_but_changes_per_tick() {
        let chunk = Vector2::new(-8, 19);
        let mut first = spawn_random(0x5eed, 400, chunk);
        let mut second = spawn_random(0x5eed, 400, chunk);
        assert_eq!(first.next_i64(), second.next_i64());
        assert_eq!(first.next_i64(), second.next_i64());

        let mut next_tick = spawn_random(0x5eed, 401, chunk);
        assert_ne!(first.next_i64(), next_tick.next_i64());
    }
}

pub fn get_top_non_colliding_pos(
    world: &World,
    cache: &dyn GenerationCache,
    entity_type: &'static EntityType,
    x: i32,
    z: i32,
) -> BlockPos {
    let mut y = cache.get_top_y(&entity_type.spawn_restriction.heightmap, x, z);
    let mut pos_vec = Vector3::new(x, y, z);
    let min_y = world.min_y;

    if world.dimension.has_ceiling {
        loop {
            y -= 1;
            pos_vec.y = y;
            // Use UFCS to avoid the ambiguity error from earlier
            if GenerationCache::get_block_state(cache, &pos_vec)
                .to_state()
                .is_air()
                || y <= min_y
            {
                break;
            }
        }

        loop {
            y -= 1;
            pos_vec.y = y;
            if !GenerationCache::get_block_state(cache, &pos_vec)
                .to_state()
                .is_air()
                || y <= min_y
            {
                break;
            }
        }
    }

    let pos = BlockPos::new(x, y, z);

    adjust_spawn_position_cache(cache, pos, entity_type)
}

pub fn spawn_category_for_position(
    category: &'static MobCategory,
    world: &Arc<World>,
    pos: BlockPos,
    _chunk_pos: &Vector2<i32>,
    spawn_state: &SpawnState,
    is_thundering: bool,
    random: &mut RandomGenerator,
) -> Vec<Arc<dyn EntityBase>> {
    let mut batch_buffer = vec![];
    let mut spawn_cluster_size = 0;
    let player_positions: Vec<_> = world.players.load().iter().map(|p| p.position()).collect();

    'group_loop: for _ in 0..3 {
        let mut new_x = pos.0.x;
        let mut new_z = pos.0.z;

        let mut random_group_size = random.next_inbetween_i32(1, 4);
        let mut inc = 0;
        let mut current_spawner = None;

        'spawn_loop: while inc < random_group_size {
            new_x += random.next_bounded_i32(6) - random.next_bounded_i32(6);
            new_z += random.next_bounded_i32(6) - random.next_bounded_i32(6);
            let mut new_pos = BlockPos::new(new_x, pos.0.y, new_z);

            if current_spawner.is_none() {
                let Some(spawner) = get_random_spawn_mob_at(world, category, &new_pos, random)
                else {
                    break 'spawn_loop;
                };
                current_spawner = Some(spawner);
                random_group_size = random.next_inbetween_i32(spawner.min_count, spawner.max_count);
            }

            let Some(spawner) = current_spawner else {
                break 'spawn_loop;
            };
            let name = spawner
                .r#type
                .strip_prefix("minecraft:")
                .unwrap_or(spawner.r#type);
            let Some(entity_type) = EntityType::from_name(name) else {
                break 'spawn_loop;
            };

            new_pos = adjust_spawn_position(world, new_pos, entity_type);

            let spawn_pos_f64 = Vector3::new(
                f64::from(new_pos.0.x) + 0.5,
                f64::from(new_pos.0.y),
                f64::from(new_pos.0.z) + 0.5,
            );

            let player_distance = get_nearest_player(&spawn_pos_f64, &player_positions);
            if !is_right_distance_to_player_and_spawn_point_for_chunk(
                world,
                &new_pos,
                player_distance,
                Some(*_chunk_pos),
            ) {
                inc += 1;
                continue;
            }

            if !is_valid_spawn_position_for_type(
                world,
                &new_pos,
                category,
                entity_type,
                player_distance,
                is_thundering,
                random,
            ) {
                inc += 1;
                continue;
            }
            if !spawn_state.can_spawn(entity_type, &new_pos, world) {
                inc += 1;
                continue;
            }

            let entity = from_type(entity_type, spawn_pos_f64, world, Uuid::new_v4());
            entity
                .get_entity()
                .set_rotation(random.next_f32() * 360., 0.);

            spawn_cluster_size += 1;
            batch_buffer.push(entity);
            spawn_state.after_spawn(entity_type, &new_pos, world);
            if spawn_cluster_size >= entity_type.limit_per_chunk {
                break 'group_loop;
            }

            inc += 1;
        }
    }
    batch_buffer
}

#[must_use]
pub fn get_nearest_player(pos: &Vector3<f64>, player_positions: &[Vector3<f64>]) -> f64 {
    let mut min_dst_sq = f64::MAX;

    for player_pos in player_positions {
        let cur_dst_sq = player_pos.squared_distance_to_vec(pos);
        if cur_dst_sq < min_dst_sq {
            min_dst_sq = cur_dst_sq;
        }
    }
    min_dst_sq
}

#[must_use]
pub fn is_right_distance_to_player_and_spawn_point(
    world: &World,
    pos: &BlockPos,
    distance: f64,
) -> bool {
    is_right_distance_to_player_and_spawn_point_for_chunk(world, pos, distance, None)
}

/// Vanilla permits a candidate in the chunk currently being processed even
/// when that chunk is not yet in the active-chunk set.  A jittered group may
/// cross into a neighbouring chunk; those candidates must use
/// `canSpawnEntitiesInChunk` (the active set in Pumpkin).  Keeping the origin
/// chunk explicit avoids accidentally allowing every neighbouring chunk.
fn is_right_distance_to_player_and_spawn_point_for_chunk(
    world: &World,
    pos: &BlockPos,
    distance: f64,
    origin_chunk: Option<Vector2<i32>>,
) -> bool {
    if distance <= 24. * 24. {
        return false;
    }

    // NaturalSpawner checks the world's actual respawn data, not the origin.
    // The previous origin shortcut incorrectly suppressed spawning near a
    // moved world spawn and allowed it near the real spawn point.  Spawn data
    // is shared by the enabled dimensions in Pumpkin, so only apply it to the
    // overworld world where vanilla's respawn dimension is defined.
    if world.dimension == pumpkin_data::dimension::Dimension::OVERWORLD {
        let info = world.level_info.load();
        let spawn = Vector3::new(
            f64::from(info.spawn_x) + 0.5,
            f64::from(info.spawn_y),
            f64::from(info.spawn_z) + 0.5,
        );
        if pos.to_centered_f64().squared_distance_to_vec(&spawn) < 24. * 24. {
            return false;
        }
    }

    // A jittered group may cross the starting chunk.  Vanilla permits the
    // attempt in the chunk currently being processed, and otherwise requires
    // `ServerLevel::canSpawnEntitiesInChunk`; active_chunks is that runtime
    // equivalent here.
    let candidate_chunk = pos.chunk_position();
    origin_chunk.is_some_and(|origin| origin == candidate_chunk)
        || world.active_chunks.load().contains(&candidate_chunk)
}

#[must_use]
pub fn get_random_spawn_mob_at(
    world: &Arc<World>,
    category: &'static MobCategory,
    block_pos: &BlockPos,
    random: &mut RandomGenerator,
) -> Option<&'static Spawner> {
    // TODO Holder<Biome> holder = level.getBiome(pos);
    let biome = world.level.get_rough_biome(block_pos);
    if category == &MobCategory::WATER_AMBIENT
        && biome.has_tag(&MINECRAFT_REDUCE_WATER_AMBIENT_SPAWNS)
        && random.next_f32() < 0.98f32
    {
        None
    } else {
        // TODO isInNetherFortressBounds(pos, level, cetagory, structureManager) then NetherFortressStructure.FORTRESS_ENEMIES
        // TODO structureManager.getAllStructuresAt(pos); ChunkGenerator::getMobsAt
        let spawners = match category.id {
            id if id == MobCategory::MONSTER.id => biome.spawners.monster,
            id if id == MobCategory::CREATURE.id => biome.spawners.creature,
            id if id == MobCategory::AMBIENT.id => biome.spawners.ambient,
            id if id == MobCategory::AXOLOTLS.id => biome.spawners.axolotls,
            id if id == MobCategory::UNDERGROUND_WATER_CREATURE.id => {
                biome.spawners.underground_water_creature
            }
            id if id == MobCategory::WATER_CREATURE.id => biome.spawners.water_creature,
            id if id == MobCategory::WATER_AMBIENT.id => biome.spawners.water_ambient,
            id if id == MobCategory::MISC.id => biome.spawners.misc,
            _ => biome.spawners.misc,
        };
        if spawners.is_empty() {
            None
        } else {
            spawners.get(random.next_bounded_i32(spawners.len() as i32) as usize)
        }
    }
}

pub fn is_valid_spawn_position_for_type(
    world: &Arc<World>,
    block_pos: &BlockPos,
    category: &'static MobCategory,
    entity_type: &'static EntityType,
    distance: f64,
    is_thundering: bool,
    random: &mut RandomGenerator,
) -> bool {
    // SpawnPlacementTypes checks the current border before fluid/ground
    // predicates.  Use the block's south-west corner, matching
    // WorldBorder.isWithinBounds(BlockPos) rather than requiring the whole
    // one-block square to fit inside the border.
    if !is_within_world_border_sync(world, block_pos) {
        return false;
    }

    // TODO !SpawnPlacements.checkSpawnRules(entityType, level, EntitySpawnReason.NATURAL, pos, level.random)
    if category == &MobCategory::MISC {
        return false;
    }
    if !entity_type.can_spawn_far_from_player
        && distance
            > f64::from(entity_type.category.despawn_distance)
                * f64::from(entity_type.category.despawn_distance)
    {
        return false;
    }
    if !entity_type.summonable {
        return false;
    }
    if !is_spawn_position_ok(world, block_pos, entity_type) {
        return false;
    }
    if !check_spawn_rules(entity_type, world, block_pos, is_thundering, random) {
        return false;
    }
    if !world.is_space_empty(BoundingBox::new_from_pos(
        f64::from(block_pos.0.x) + 0.5,
        f64::from(block_pos.0.y),
        f64::from(block_pos.0.z) + 0.5,
        &natural_spawn_dimensions(entity_type),
    )) {
        return false;
    }
    true
}

/// Synchronous counterpart used by chunk-generation spawning.  World border
/// commands briefly hold the async mutex; if that happens while a generation
/// worker is probing a candidate, leave the candidate for the next pass rather
/// than blocking the generation thread on an async lock.
pub(crate) fn is_within_world_border_sync(world: &World, block_pos: &BlockPos) -> bool {
    world
        .worldborder
        .try_lock()
        .map(|border| border.contains(f64::from(block_pos.0.x), f64::from(block_pos.0.z)))
        // A command may briefly hold the async mutex while changing the
        // border.  Rejecting this one synchronous candidate is conservative
        // (the next spawn pass retries it); allowing it through could violate
        // the newly reduced border.
        .unwrap_or(false)
}

/// Vanilla `Mob#checkDespawn` decision for a naturally spawned mob. The
/// nearest-player lookup is intentionally outside this pure helper: callers
/// must exclude spectators and may provide the exact squared distance they
/// observed in the current world tick. Persistent, named, or leashed mobs
/// are never removed by distance despawn.
#[must_use]
pub fn should_despawn_mob(
    entity_type: &'static EntityType,
    persistence_required: bool,
    has_custom_name: bool,
    is_leashed: bool,
    nearest_player_distance_squared: Option<f64>,
    random_roll: u16,
) -> bool {
    if !entity_type.mob
        || entity_type.category == &MobCategory::MISC
        || persistence_required
        || has_custom_name
        || is_leashed
    {
        return false;
    }

    let Some(distance_squared) = nearest_player_distance_squared else {
        // `ServerLevel#getNearestPlayer` returns no candidate outside the
        // category's despawn radius; without a candidate the mob stays alive.
        return false;
    };
    let despawn_distance = f64::from(entity_type.category.despawn_distance);
    if distance_squared > despawn_distance * despawn_distance {
        return true;
    }

    distance_squared > f64::from(MobCategory::NO_DESPAWN_DISTANCE).powi(2) && random_roll == 0
}

/// Produces the per-tick 0..=799 despawn roll without touching a process
/// global RNG.  Vanilla consumes `ServerLevel.random` while ticking entities;
/// Pumpkin ticks entities concurrently, so deriving the roll from the
/// authoritative world age and stable entity id avoids a scheduling-dependent
/// result while preserving the one-in-800 probability.
#[must_use]
pub const fn natural_despawn_roll(world_age: i64, entity_id: i32) -> u16 {
    let mut value = (world_age as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(entity_id as u32 as u64)
        .wrapping_add(0xD1B5_4A32_D192_ED03);
    value ^= value >> 30;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^= value >> 31;
    (value % 800) as u16
}

pub fn is_spawn_position_ok(
    world: &Arc<World>,
    block_pos: &BlockPos,
    entity_type: &'static EntityType,
) -> bool {
    match entity_type.spawn_restriction.location {
        SpawnLocation::InLava => world.get_fluid(block_pos).has_tag(&MINECRAFT_LAVA),
        SpawnLocation::InWater => {
            let above_state = world.get_block_state(&block_pos.up());
            // Vanilla's default isRedstoneConductor predicate is
            // isCollisionShapeFullBlock.  Use the generated collision shape,
            // not the coarser solid-block flag, so slabs, stairs and
            // waterlogged states remain valid spawn spaces.
            world.get_fluid(block_pos).has_tag(&MINECRAFT_WATER)
                && !above_state.is_collision_shape_full_block()
        }
        SpawnLocation::OnGround => {
            let down = world.get_block_state(&block_pos.down());
            let up = world.get_block_state(&block_pos.up());
            let cur = world.get_block_state(block_pos);
            // SpawnPlacementTypes.ON_GROUND delegates to the support state's
            // per-block `isValidSpawn` predicate.  Pumpkin does not yet expose
            // that predicate in generated block metadata, so side sturdiness is
            // the conservative common denominator; entity-specific rules below
            // still decide the biome/light/difficulty restrictions.
            let is_valid_spawn_below = is_valid_spawn_support(down);

            if is_valid_spawn_below {
                is_valid_empty_spawn_block_for_type(cur, entity_type)
                    && is_valid_empty_spawn_block_for_type(up, entity_type)
            } else {
                false
            }
        }
        SpawnLocation::Unrestricted => true,
    }
}

/// Cache-based version of `is_spawn_position_ok` used during world generation.
pub fn is_spawn_position_ok_cache(
    cache: &dyn GenerationCache,
    block_pos: &BlockPos,
    entity_type: &'static EntityType,
) -> bool {
    let pos_vec = block_pos.0;
    let state = GenerationCache::get_block_state(cache, &pos_vec).to_state();

    match entity_type.spawn_restriction.location {
        SpawnLocation::InLava => {
            // During generation, we check the block state's liquid property and tag
            state.is_liquid() && Block::from_state_id(state.id).has_tag(&MINECRAFT_LAVA)
        }
        SpawnLocation::InWater => {
            let above_pos = block_pos.up().0;
            let above_state = GenerationCache::get_block_state(cache, &above_pos).to_state();

            state.is_liquid()
                && Block::from_state_id(state.id).has_tag(&MINECRAFT_WATER)
                && !above_state.is_collision_shape_full_block()
        }
        SpawnLocation::OnGround => {
            let down_pos = block_pos.down().0;
            let up_pos = block_pos.up().0;

            let down = GenerationCache::get_block_state(cache, &down_pos).to_state();
            let up = GenerationCache::get_block_state(cache, &up_pos).to_state();

            // Keep this in sync with the runtime path.  The generated cache has
            // no per-block `isValidSpawn` callback, so use side sturdiness until
            // that metadata is modelled explicitly.
            let is_valid_spawn_below = is_valid_spawn_support(&down);

            if is_valid_spawn_below {
                is_valid_empty_spawn_block_for_type(state, entity_type)
                    && is_valid_empty_spawn_block_for_type(up, entity_type)
            } else {
                false
            }
        }
        SpawnLocation::Unrestricted => true,
    }
}

/// Cache-based version of `adjust_spawn_position` used during world generation.
pub fn adjust_spawn_position_cache(
    cache: &dyn GenerationCache,
    pos: BlockPos,
    entity_type: &'static EntityType,
) -> BlockPos {
    if matches!(
        entity_type.spawn_restriction.location,
        SpawnLocation::OnGround
    ) {
        let below = pos.down();
        let state = GenerationCache::get_block_state(cache, &below.0).to_state();

        if !state.is_collision_shape_full_block() && !state.is_liquid() {
            return below;
        }
    }
    pos
}

pub fn adjust_spawn_position(
    world: &World,
    pos: BlockPos,
    entity_type: &'static EntityType,
) -> BlockPos {
    if matches!(
        entity_type.spawn_restriction.location,
        SpawnLocation::OnGround
    ) {
        let below = pos.down();
        let state = world.get_block_state(&below);
        // Approximation of isPathfindable(LAND)
        if !state.is_collision_shape_full_block() && !state.is_liquid() {
            return below;
        }
    }
    pos
}

#[must_use]
pub fn is_valid_empty_spawn_block(state: &'static BlockState) -> bool {
    is_valid_empty_spawn_block_for_type(state, &EntityType::ZOMBIE)
}

/// Vanilla `EntityType#getSpawnAABB` applies a four-times spawn-dimension scale
/// to slime and magma-cube candidates.  The runtime entity later picks a
/// random size, but the pre-spawn obstruction check must use this conservative
/// maximum box or a large variant can be created inside an occupied volume.
#[must_use]
fn natural_spawn_dimensions(entity_type: &'static EntityType) -> EntityDimensions {
    let scale = if entity_type == &EntityType::SLIME || entity_type == &EntityType::MAGMA_CUBE {
        4.0
    } else {
        1.0
    };
    EntityDimensions {
        width: entity_type.dimension[0] * scale,
        height: entity_type.dimension[1] * scale,
        eye_height: entity_type.eye_height * scale,
    }
}

/// Entity-aware counterpart of `NaturalSpawner.isValidEmptySpawnBlock`.
/// Fire-immune types may occupy fire/soul-fire blocks; all other dangerous
/// blocks remain rejected.  The legacy wrapper above intentionally keeps the
/// default hostile-mob behaviour for callers and tests that do not have a type.
#[must_use]
fn is_valid_empty_spawn_block_for_type(
    state: &'static BlockState,
    entity_type: &'static EntityType,
) -> bool {
    // NaturalSpawner.isValidEmptySpawnBlock uses the actual collision shape,
    // not the generated full-cube/render flag.  This matters for trapdoors,
    // slabs, panes, and custom states whose collision and rendering metadata
    // intentionally differ.
    if state.is_collision_shape_full_block() {
        return false;
    }
    if is_signal_source(state) {
        return false;
    }
    if state.is_liquid() {
        return false;
    }
    if Block::from_state_id(state.id).has_tag(&MINECRAFT_PREVENT_MOB_SPAWNING_INSIDE) {
        return false;
    }

    // EntityType#isBlockDangerous is subtype data in Mojang.  These blocks are
    // dangerous for every naturally spawned mob and must never be treated as
    // an empty spawn volume by the generic path.
    let block = Block::from_state_id(state.id);
    (entity_type.fire_immune || (block != &Block::FIRE && block != &Block::SOUL_FIRE))
        && block != &Block::SWEET_BERRY_BUSH
        && block != &Block::WITHER_ROSE
        && block != &Block::CACTUS
        && block != &Block::POWDER_SNOW
}

/// Default `BlockBehaviour` implementation of `BlockState.isValidSpawn` for
/// the support block below an on-ground mob.  The vanilla predicate requires
/// an upward sturdy face and light emission below 14; using only side
/// sturdiness would incorrectly allow mobs on maximum-luminance blocks such
/// as glowstone and sea lanterns.
#[must_use]
fn is_valid_spawn_support(state: &BlockState) -> bool {
    state.is_side_solid(BlockDirection::Up) && state.luminance < 14
}

/// Vanilla's `NaturalSpawner.isValidEmptySpawnBlock` rejects every state from
/// a block whose behaviour is a redstone signal source.  That predicate lives
/// on Pumpkin's behaviour registry and is asynchronous, while spawning has to
/// make this decision synchronously for thousands of candidates.  Keep the
/// registry-independent part here using the generated vanilla tags and the
/// small set of non-tagged signal-source blocks.
#[must_use]
fn is_signal_source(state: &'static BlockState) -> bool {
    let block = Block::from_state_id(state.id);
    block.has_tag(&tag::Block::MINECRAFT_BUTTONS)
        || block.has_tag(&tag::Block::MINECRAFT_PRESSURE_PLATES)
        || block == &Block::COMPARATOR
        || block == &Block::DAYLIGHT_DETECTOR
        || block == &Block::DETECTOR_RAIL
        || block == &Block::LIGHTNING_ROD
        || block == &Block::OBSERVER
        || block == &Block::REDSTONE_BLOCK
        || block == &Block::REDSTONE_TORCH
        || block == &Block::REPEATER
        || block == &Block::REDSTONE_WIRE
        || block == &Block::SCULK_SENSOR
        || block == &Block::CALIBRATED_SCULK_SENSOR
        || block == &Block::TARGET
        || block == &Block::TRIPWIRE_HOOK
        || block == &Block::LEVER
}

#[cfg(test)]
mod tests {
    use super::{
        PointCharge, is_signal_source, is_valid_empty_spawn_block,
        is_valid_empty_spawn_block_for_type, is_valid_spawn_support, natural_despawn_roll,
        natural_spawn_dimensions, should_despawn_mob,
    };
    use pumpkin_data::Block;
    use pumpkin_data::entity::EntityType;
    use pumpkin_util::math::position::BlockPos;

    #[test]
    fn dangerous_blocks_are_not_empty_spawn_volumes() {
        for block in [
            &Block::FIRE,
            &Block::SOUL_FIRE,
            &Block::SWEET_BERRY_BUSH,
            &Block::WITHER_ROSE,
            &Block::CACTUS,
        ] {
            assert!(!is_valid_empty_spawn_block(block.default_state));
        }
        assert!(is_valid_empty_spawn_block(Block::AIR.default_state));
        assert!(!is_valid_empty_spawn_block(
            Block::POWDER_SNOW.default_state
        ));
    }

    #[test]
    fn spawn_dimensions_match_entity_type_scale() {
        let zombie = natural_spawn_dimensions(&EntityType::ZOMBIE);
        assert_eq!(zombie.width, EntityType::ZOMBIE.dimension[0]);
        assert_eq!(zombie.height, EntityType::ZOMBIE.dimension[1]);

        let slime = natural_spawn_dimensions(&EntityType::SLIME);
        assert_eq!(slime.width, EntityType::SLIME.dimension[0] * 4.0);
        assert_eq!(slime.height, EntityType::SLIME.dimension[1] * 4.0);
    }

    #[test]
    fn fire_immune_spawn_types_follow_vanilla_dangerous_block_rules() {
        assert!(!is_valid_empty_spawn_block_for_type(
            Block::FIRE.default_state,
            &EntityType::ZOMBIE,
        ));
        assert!(is_valid_empty_spawn_block_for_type(
            Block::FIRE.default_state,
            &EntityType::MAGMA_CUBE,
        ));
        assert!(!is_valid_empty_spawn_block_for_type(
            Block::POWDER_SNOW.default_state,
            &EntityType::MAGMA_CUBE,
        ));
    }

    #[test]
    fn redstone_signal_sources_are_not_empty_spawn_volumes() {
        for block in [
            &Block::REDSTONE_BLOCK,
            &Block::REDSTONE_TORCH,
            &Block::REPEATER,
            &Block::TARGET,
            &Block::LEVER,
        ] {
            assert!(is_signal_source(block.default_state));
            assert!(!is_valid_empty_spawn_block(block.default_state));
        }
        assert!(!is_signal_source(Block::AIR.default_state));
    }

    #[test]
    fn on_ground_support_matches_default_vanilla_predicate() {
        assert!(is_valid_spawn_support(Block::GRASS_BLOCK.default_state));
        assert!(!is_valid_spawn_support(Block::GLOWSTONE.default_state));
        assert!(!is_valid_spawn_support(Block::AIR.default_state));
    }

    #[test]
    fn coincident_spawn_charge_has_finite_zero_potential() {
        let charge = PointCharge(BlockPos::new(3, 64, -2).to_f64(), 12.0);
        assert_eq!(charge.get_potential_change(&BlockPos::new(3, 64, -2)), 0.0);
    }

    #[test]
    fn natural_mob_despawn_respects_persistence_name_leash_and_distance() {
        let zombie = &EntityType::ZOMBIE;
        assert!(!should_despawn_mob(
            zombie,
            true,
            false,
            false,
            Some(200.0),
            0
        ));
        assert!(!should_despawn_mob(
            zombie,
            false,
            true,
            false,
            Some(200.0),
            0
        ));
        assert!(!should_despawn_mob(
            zombie,
            false,
            false,
            true,
            Some(200.0),
            0
        ));
        assert!(should_despawn_mob(
            zombie,
            false,
            false,
            false,
            Some(129.0 * 129.0),
            1
        ));
        assert!(!should_despawn_mob(
            zombie,
            false,
            false,
            false,
            Some(33.0 * 33.0),
            1
        ));
        assert!(should_despawn_mob(
            zombie,
            false,
            false,
            false,
            Some(33.0 * 33.0),
            0
        ));
    }

    #[test]
    fn natural_despawn_roll_is_stable_per_tick_and_entity() {
        assert_eq!(natural_despawn_roll(20, 42), natural_despawn_roll(20, 42));
        assert_ne!(natural_despawn_roll(20, 42), natural_despawn_roll(21, 42));
        assert_ne!(natural_despawn_roll(20, 42), natural_despawn_roll(20, 43));
        assert!(natural_despawn_roll(20, 42) < 800);
    }
}
