use std::{
    pin::Pin,
    sync::{
        Arc, RwLock,
        atomic::{AtomicI32, Ordering},
    },
};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::{entity::EntityType, world::WorldEvent};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{
    boundingbox::{BoundingBox, EntityDimensions},
    position::BlockPos,
    vector3::Vector3,
};

use crate::entity::r#type::check_spawn_rules;
use crate::world::natural_spawner::{is_spawn_position_ok, is_within_world_border_sync};
use crate::{block::entities::BlockEntity, world::World};
use pumpkin_util::random::{RandomGenerator, RandomImpl, xoroshiro128::Xoroshiro};

/// One weighted entry from a spawner's `SpawnPotentials` list.
///
/// The complete entity compound is retained instead of only the registry id.
/// This is important for vanilla spawners whose entries carry custom entity
/// data (for example a specific horse variant, equipment, or a baby flag).
#[derive(Clone, Debug)]
struct SpawnPotential {
    weight: i32,
    entity_type: Option<&'static EntityType>,
    data: NbtCompound,
}

fn entity_data_from_spawn_data(data: &NbtCompound) -> Option<&NbtCompound> {
    data.get_compound("entity")
        .or_else(|| data.get_compound("Entity"))
        .or_else(|| data.get_string("id").map(|_| data))
}

fn entity_type_from_data(data: &NbtCompound) -> Option<&'static EntityType> {
    let entity = entity_data_from_spawn_data(data)?;
    let id = entity
        .get_string("id")
        .or_else(|| entity.get_string("Id"))
        .or_else(|| data.get_string("Type"))?;
    EntityType::from_name(id.strip_prefix("minecraft:").unwrap_or(id))
}

fn weighted_index(entries: &[SpawnPotential], ticket: i32) -> Option<usize> {
    let total = entries
        .iter()
        .map(|entry| entry.weight.max(0))
        .fold(0_i32, i32::saturating_add);
    if total <= 0 {
        return None;
    }
    let mut cursor = ticket.rem_euclid(total);
    for (index, entry) in entries.iter().enumerate() {
        let weight = entry.weight.max(0);
        if cursor < weight {
            return Some(index);
        }
        cursor -= weight;
    }
    None
}

fn get_i32(nbt: &NbtCompound, name: &str, default: i32) -> i32 {
    nbt.get_int(name)
        .or_else(|| nbt.get_short(name).map(i32::from))
        .unwrap_or(default)
}

/// BaseSpawner uses a half-open random interval for its next delay.  Keeping
/// this in one helper matches `minSpawnDelay + random.nextInt(max-min)` from
/// Mojang's `BaseSpawner#delay`: the configured maximum is an exclusive bound.
#[inline]
fn random_delay(min_delay: i32, max_delay: i32) -> i32 {
    if max_delay <= min_delay {
        min_delay
    } else {
        min_delay + rand::random_range(0..(max_delay - min_delay))
    }
}

/// Mirrors `SpawnUtil.trySpawnMob(..., ON_TOP_OF_COLLIDER, true)` used by
/// Mojang's `BaseSpawner`.  A spawner gets three random X/Z attempts per
/// requested mob, searches one block up/down for a valid full support face,
/// then applies the entity's placement and mob-specific spawn predicates
/// before creating the entity.  The previous single floating-point probe
/// could place mobs inside walls or let a spawner bypass darkness/water rules.
fn find_spawn_position(
    world: &Arc<World>,
    origin: BlockPos,
    entity_type: &'static EntityType,
    spawn_range: i32,
    random: &mut RandomGenerator,
) -> Option<BlockPos> {
    let range = spawn_range.max(0);
    for _ in 0..3 {
        let x = origin.0.x + random.next_inbetween_i32(-range, range);
        let z = origin.0.z + random.next_inbetween_i32(-range, range);
        let mut search = BlockPos::new(x, origin.0.y + 1, z);
        let mut above_state = world.get_block_state(&search);

        for _ in 0..=2 {
            search = search.down();
            let current_state = world.get_block_state(&search);
            let spawn_pos = search.up();
            let above_empty = above_state.get_block_collision_shapes().next().is_none();
            let supported = current_state.is_collision_shape_full_block();
            if above_empty
                && supported
                && is_within_world_border_sync(world, &spawn_pos)
                && is_spawn_position_ok(world, &spawn_pos, entity_type)
                && check_spawn_rules(entity_type, world, &spawn_pos, false, random)
                && world.is_space_empty(BoundingBox::new_from_pos(
                    f64::from(spawn_pos.0.x) + 0.5,
                    f64::from(spawn_pos.0.y),
                    f64::from(spawn_pos.0.z) + 0.5,
                    &EntityDimensions {
                        width: entity_type.dimension[0],
                        height: entity_type.dimension[1],
                        eye_height: entity_type.eye_height,
                    },
                ))
            {
                return Some(spawn_pos);
            }
            above_state = current_state;
        }
    }
    None
}

pub struct MobSpawnerBlockEntity {
    /// Current position used by the spawner.  It is mutable because the same
    /// state machine is shared by block spawners and moving spawner minecarts.
    pub position: AtomicCell<BlockPos>,
    pub delay: AtomicI32,
    pub max_delay: i32,
    pub min_delay: i32,
    pub spawn_count: i32,
    pub spawn_range: i32,
    pub max_nearby_entities: i32,
    pub required_player_range: i32,
    pub entity_type: AtomicCell<Option<&'static EntityType>>,
    spawn_data: RwLock<Option<NbtCompound>>,
    spawn_potentials: RwLock<Vec<SpawnPotential>>,
}

impl MobSpawnerBlockEntity {
    pub const ID: &'static str = "minecraft:mob_spawner";
    pub const DEFAULT_DELAY: i32 = 20;
    pub const DEFAULT_MAX_SPAWN_DELAY: i32 = 800;
    pub const DEFAULT_MIN_SPAWN_DELAY: i32 = 200;
    pub const DEFAULT_SPAWN_COUNT: i32 = 4;
    pub const DEFAULT_SPAWN_RANGE: i32 = 4;
    pub const DEFAULT_MAX_NEARBY_ENTITIES: i32 = 6;
    pub const DEFAULT_REQUIRED_PLAYER_RANGE: i32 = 16;

    /// A spawner is ready as soon as its delay reaches zero.  Vanilla's
    /// server tick decrements positive delays and performs the spawn in the
    /// same tick that observes zero; using a sentinel such as `-1` adds an
    /// extra 1-tick gap and makes malformed negative NBT values spin forever.
    #[inline]
    const fn is_ready(delay: i32) -> bool {
        delay <= 0
    }

    #[must_use]
    pub fn new(position: BlockPos, entity_type: Option<&'static EntityType>) -> Self {
        Self {
            position: AtomicCell::new(position),
            delay: AtomicI32::new(Self::DEFAULT_DELAY),
            max_delay: Self::DEFAULT_MAX_SPAWN_DELAY,
            min_delay: Self::DEFAULT_MIN_SPAWN_DELAY,
            spawn_count: Self::DEFAULT_SPAWN_COUNT,
            spawn_range: Self::DEFAULT_SPAWN_RANGE,
            max_nearby_entities: Self::DEFAULT_MAX_NEARBY_ENTITIES,
            required_player_range: Self::DEFAULT_REQUIRED_PLAYER_RANGE,
            entity_type: AtomicCell::new(entity_type),
            spawn_data: RwLock::new(entity_type.map(|entity_type| {
                let mut data = NbtCompound::new();
                let mut entity = NbtCompound::new();
                entity.put_string("id", format!("minecraft:{}", entity_type.resource_name));
                data.put_compound("entity", entity);
                data
            })),
            spawn_potentials: RwLock::new(Vec::new()),
        }
    }

    pub fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) {
        // TODO: this is ugly af
        nbt.put_string("id", self.resource_location().to_string());
        let position = self.get_position();
        nbt.put_int("x", position.0.x);
        nbt.put_int("y", position.0.y);
        nbt.put_int("z", position.0.z);
        nbt.put_short("Delay", self.delay.load(Ordering::Relaxed) as i16);
        nbt.put_short(
            "MinSpawnDelay",
            self.min_delay.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        );
        nbt.put_short(
            "MaxSpawnDelay",
            self.max_delay.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        );
        nbt.put_short(
            "SpawnCount",
            self.spawn_count.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        );
        nbt.put_short(
            "MaxNearbyEntities",
            self.max_nearby_entities
                .clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        );
        nbt.put_short(
            "RequiredPlayerRange",
            self.required_player_range
                .clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        );
        nbt.put_short(
            "SpawnRange",
            self.spawn_range.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        );
        if let Some(spawn_data) = self
            .spawn_data
            .read()
            .expect("mob spawner spawn data lock poisoned")
            .clone()
        {
            nbt.put_compound("SpawnData", spawn_data);
        } else if let Some(entity_type) = self.entity_type.load() {
            let mut entity = NbtCompound::new();
            entity.put_string("id", format!("minecraft:{}", entity_type.resource_name));
            let mut spawn_data = NbtCompound::new();
            spawn_data.put_compound("entity", entity);
            nbt.put_compound("SpawnData", spawn_data);
        }
        let potentials = self
            .spawn_potentials
            .read()
            .expect("mob spawner potentials lock poisoned");
        if !potentials.is_empty() {
            let list = potentials
                .iter()
                .map(|potential| {
                    let mut entry = NbtCompound::new();
                    entry.put_int("weight", potential.weight.max(0));
                    entry.put_compound("data", potential.data.clone());
                    pumpkin_nbt::tag::NbtTag::Compound(entry)
                })
                .collect();
            nbt.put_list("SpawnPotentials", list);
        }
    }

    /// Writes only the `BaseSpawner` payload used by an entity-backed spawner
    /// (for example a spawner minecart). Block entities additionally need
    /// `id`/`x`/`y`/`z`, but those keys are not part of an entity's NBT root.
    pub fn write_entity_nbt(&self, nbt: &mut NbtCompound) {
        let mut serialized = NbtCompound::new();
        self.write_nbt(&mut serialized);
        for key in ["id", "x", "y", "z"] {
            serialized.child_tags.remove(key);
        }
        nbt.child_tags.extend(serialized.child_tags);
    }
}

impl MobSpawnerBlockEntity {
    async fn update_spawns(&self, world: &Arc<World>) {
        let min_delay = self.min_delay;
        let max_delay = self.max_delay;

        self.delay
            .store(random_delay(min_delay, max_delay), Ordering::Relaxed);
        if !self
            .spawn_potentials
            .read()
            .expect("mob spawner potentials lock poisoned")
            .is_empty()
        {
            let potentials = self
                .spawn_potentials
                .read()
                .expect("mob spawner potentials lock poisoned");
            let total = potentials
                .iter()
                .map(|entry| entry.weight.max(0))
                .fold(0_i32, i32::saturating_add);
            if total > 0 {
                let ticket = rand::random_range(0..total);
                if let Some(index) = weighted_index(&potentials, ticket) {
                    let selected = &potentials[index];
                    self.entity_type.store(selected.entity_type);
                    *self
                        .spawn_data
                        .write()
                        .expect("mob spawner spawn data lock poisoned") =
                        Some(selected.data.clone());
                }
            }
        }
        world
            .add_synced_block_event(self.position.load(), 1, 0)
            .await;
    }

    pub fn set_entity_type(&self, entity_type: &'static EntityType) {
        self.entity_type.store(Some(entity_type));
        let mut data = NbtCompound::new();
        let mut entity = NbtCompound::new();
        entity.put_string("id", format!("minecraft:{}", entity_type.resource_name));
        data.put_compound("entity", entity);
        *self
            .spawn_data
            .write()
            .expect("mob spawner spawn data lock poisoned") = Some(data);
    }

    /// Move the spawner without resetting its delay or weighted potentials.
    pub fn set_position(&self, position: BlockPos) {
        self.position.store(position);
    }
}

impl BlockEntity for MobSpawnerBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position.load()
    }

    fn tick<'a>(&'a self, world: &'a Arc<World>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            // Java's BaseSpawner is gated by the `spawner_blocks_work`
            // gamerule.  Keep the delay and weighted state untouched while
            // disabled: toggling the rule off must pause a spawner, not reset
            // its countdown or discard its selected SpawnData.
            if !world.level_info.load().game_rules.spawner_blocks_work {
                return;
            }
            let Some(entity_type) = self.entity_type.load() else {
                return;
            };
            let position = self.position.load();
            let center = position.to_f64().add_raw(0.5, 0.5, 0.5);
            if world
                .get_nearby_players(center, f64::from(self.required_player_range))
                .is_empty()
            {
                return;
            }
            // BaseSpawner queries an inflated block AABB, not a sphere around
            // the block centre.  The distinction matters for mobs near a
            // corner of the 18x18x18 cap volume: using the radial helper
            // admitted fewer entities and allowed a spawner to overproduce.
            let nearby_box = BoundingBox::from_block(&position).expand_all(9.0);
            let nearby_same_type = world
                .get_entities_at_box(&nearby_box)
                .into_iter()
                .filter(|entity| {
                    entity.get_entity().is_alive()
                        && entity.get_entity().entity_type.id == entity_type.id
                })
                .count() as i32;
            if nearby_same_type >= self.max_nearby_entities {
                self.delay.store(1, Ordering::Relaxed);
                return;
            }
            {
                let delay = self.delay.load(Ordering::Relaxed);
                if !Self::is_ready(delay) {
                    self.delay.store(delay - 1, Ordering::Relaxed);
                    return;
                }
                let spawn_range = self.spawn_range;
                let mut random = RandomGenerator::Xoroshiro(Xoroshiro::from_seed(rand::random()));
                let mut update_spawns = false;
                for _ in 0..self.spawn_count {
                    let Some(spawn_block) =
                        find_spawn_position(world, position, entity_type, spawn_range, &mut random)
                    else {
                        continue;
                    };
                    let spawn_pos = Vector3::new(
                        f64::from(spawn_block.0.x) + 0.5,
                        f64::from(spawn_block.0.y),
                        f64::from(spawn_block.0.z) + 0.5,
                    );
                    let entity = crate::entity::r#type::from_type(
                        entity_type,
                        spawn_pos,
                        world,
                        uuid::Uuid::new_v4(),
                    );
                    let spawn_entity_data = self
                        .spawn_data
                        .read()
                        .expect("mob spawner spawn data lock poisoned")
                        .as_ref()
                        .and_then(entity_data_from_spawn_data)
                        .cloned();
                    if let Some(spawn_data) = spawn_entity_data.as_ref() {
                        entity.read_nbt_non_mut(spawn_data).await;
                    }
                    world.spawn_entity(entity).await;
                    world.sync_world_event(WorldEvent::ParticlesMobblockSpawn, position, 0);
                    update_spawns = true;
                }
                if update_spawns {
                    self.update_spawns(world).await;
                }
            }
        })
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let delay = nbt.get_short("Delay").unwrap_or(Self::DEFAULT_DELAY as i16) as i32;
        let min_delay = get_i32(nbt, "MinSpawnDelay", Self::DEFAULT_MIN_SPAWN_DELAY).max(0);
        let max_delay = get_i32(nbt, "MaxSpawnDelay", Self::DEFAULT_MAX_SPAWN_DELAY).max(min_delay);
        let spawn_count = get_i32(nbt, "SpawnCount", Self::DEFAULT_SPAWN_COUNT).max(0);
        let spawn_range = get_i32(nbt, "SpawnRange", Self::DEFAULT_SPAWN_RANGE).max(0);
        let max_nearby_entities =
            get_i32(nbt, "MaxNearbyEntities", Self::DEFAULT_MAX_NEARBY_ENTITIES).max(0);
        let required_player_range = get_i32(
            nbt,
            "RequiredPlayerRange",
            Self::DEFAULT_REQUIRED_PLAYER_RANGE,
        )
        .max(0);

        let spawn_data = nbt.get_compound("SpawnData").cloned();
        let mut potentials = nbt
            .get_list("SpawnPotentials")
            .into_iter()
            .flat_map(|entries| entries.iter())
            .filter_map(|tag| tag.extract_compound())
            .filter_map(|entry| {
                let data = entry
                    .get_compound("data")
                    .or_else(|| entry.get_compound("Entity"))
                    .cloned()
                    .or_else(|| {
                        entry.get_string("Type").map(|entity_id| {
                            let mut entity = NbtCompound::new();
                            entity.put_string("id", entity_id.to_string());
                            let mut data = NbtCompound::new();
                            data.put_compound("entity", entity);
                            data
                        })
                    })?;
                Some(SpawnPotential {
                    weight: entry.get_int("weight").unwrap_or(1).max(0),
                    entity_type: entity_type_from_data(&data),
                    data,
                })
            })
            .collect::<Vec<_>>();
        if potentials.is_empty() {
            if let Some(data) = spawn_data.as_ref() {
                if let Some(entity_type) = entity_type_from_data(data) {
                    potentials.push(SpawnPotential {
                        weight: 1,
                        entity_type: Some(entity_type),
                        data: data.clone(),
                    });
                }
            }
        }
        let (entity_type, selected_spawn_data) = if let Some(data) = spawn_data {
            (entity_type_from_data(&data), Some(data))
        } else {
            let total = potentials
                .iter()
                .map(|potential| potential.weight.max(0))
                .fold(0_i32, i32::saturating_add);
            let selected = (total > 0)
                .then(|| rand::random_range(0..total))
                .and_then(|ticket| weighted_index(&potentials, ticket))
                .map(|index| potentials[index].data.clone());
            (
                selected
                    .as_ref()
                    .and_then(entity_type_from_data)
                    .or_else(|| potentials.iter().find_map(|entry| entry.entity_type)),
                selected,
            )
        };

        Self {
            position: AtomicCell::new(position),
            delay: AtomicI32::new(delay),
            max_delay,
            min_delay,
            spawn_count,
            spawn_range,
            max_nearby_entities,
            required_player_range,
            entity_type: AtomicCell::new(entity_type),
            spawn_data: RwLock::new(selected_spawn_data),
            spawn_potentials: RwLock::new(potentials),
        }
    }

    fn write_nbt<'a>(
        &'a self,
        nbt: &'a mut NbtCompound,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            self.write_nbt(nbt);
        })
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut final_nbt = NbtCompound::new();
        if let Some(spawn_data) = self
            .spawn_data
            .read()
            .expect("mob spawner spawn data lock poisoned")
            .clone()
        {
            final_nbt.put_compound("SpawnData", spawn_data);
        } else if let Some(entity_type) = self.entity_type.load() {
            let mut entity = NbtCompound::new();
            entity.put_string("id", format!("minecraft:{}", entity_type.resource_name));
            let mut spawn_entry = NbtCompound::new();
            spawn_entry.put_compound("entity", entity);
            final_nbt.put_compound("SpawnData", spawn_entry);
        }
        Some(final_nbt)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{MobSpawnerBlockEntity, SpawnPotential, random_delay, weighted_index};
    use crate::block::entities::BlockEntity;
    use pumpkin_nbt::compound::NbtCompound;
    use pumpkin_nbt::tag::NbtTag;
    use pumpkin_util::math::position::BlockPos;

    #[test]
    fn spawner_is_ready_at_zero_and_for_negative_legacy_values() {
        assert!(!MobSpawnerBlockEntity::is_ready(1));
        assert!(MobSpawnerBlockEntity::is_ready(0));
        assert!(MobSpawnerBlockEntity::is_ready(-1));
    }

    #[test]
    fn spawner_delay_uses_vanilla_half_open_bounds() {
        assert_eq!(random_delay(20, 20), 20);
        assert_eq!(random_delay(30, 10), 30);
        for _ in 0..256 {
            let delay = random_delay(10, 12);
            assert!((10..12).contains(&delay));
        }
    }

    #[test]
    fn spawn_potentials_use_non_negative_weighted_ranges() {
        let data = NbtCompound::new();
        let entries = vec![
            SpawnPotential {
                weight: 0,
                entity_type: None,
                data: data.clone(),
            },
            SpawnPotential {
                weight: 2,
                entity_type: None,
                data: data.clone(),
            },
            SpawnPotential {
                weight: 1,
                entity_type: None,
                data,
            },
        ];
        assert_eq!(weighted_index(&entries, 0), Some(1));
        assert_eq!(weighted_index(&entries, 1), Some(1));
        assert_eq!(weighted_index(&entries, 2), Some(2));
        assert_eq!(weighted_index(&entries, 3), Some(1));
        assert_eq!(weighted_index(&entries, -1), Some(2));
    }

    #[test]
    fn spawn_potentials_round_trip_without_losing_entity_data() {
        let mut zombie = NbtCompound::new();
        zombie.put_string("id", "minecraft:zombie".to_string());
        zombie.put_bool("IsBaby", true);
        let mut spawn_data = NbtCompound::new();
        spawn_data.put_compound("entity", zombie.clone());

        let mut skeleton = NbtCompound::new();
        skeleton.put_string("id", "minecraft:skeleton".to_string());
        let mut weighted = NbtCompound::new();
        weighted.put_int("weight", 3);
        weighted.put_compound("data", {
            let mut data = NbtCompound::new();
            data.put_compound("entity", skeleton);
            data
        });

        let mut source = NbtCompound::new();
        source.put_short("Delay", 7);
        source.put_compound("SpawnData", spawn_data);
        source.put_list("SpawnPotentials", vec![NbtTag::Compound(weighted)]);

        let restored = MobSpawnerBlockEntity::from_nbt(&source, BlockPos::new(1, 2, 3));
        let mut saved = NbtCompound::new();
        restored.write_nbt(&mut saved);

        assert_eq!(saved.get_short("Delay"), Some(7));
        assert_eq!(
            saved.get_compound("SpawnData").and_then(|data| {
                data.get_compound("entity")
                    .and_then(|entity| entity.get_string("id"))
            }),
            Some("minecraft:zombie")
        );
        let potentials = saved
            .get_list("SpawnPotentials")
            .expect("SpawnPotentials must be retained");
        assert_eq!(potentials.len(), 1);
        let entry = potentials[0].extract_compound().expect("compound entry");
        assert_eq!(entry.get_int("weight"), Some(3));
        assert_eq!(
            entry.get_compound("data").and_then(|data| {
                data.get_compound("entity")
                    .and_then(|entity| entity.get_string("id"))
            }),
            Some("minecraft:skeleton")
        );
    }

    #[test]
    fn entity_spawner_payload_omits_block_entity_coordinates() {
        let spawner = MobSpawnerBlockEntity::new(BlockPos::new(4, 5, 6), None);
        let mut saved = NbtCompound::new();
        spawner.write_entity_nbt(&mut saved);
        assert!(saved.get_string("id").is_none());
        assert!(saved.get_int("x").is_none());
        assert!(saved.get_int("y").is_none());
        assert!(saved.get_int("z").is_none());
        assert!(saved.get_short("Delay").is_some());
    }
}
