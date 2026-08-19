use super::BlockEntity;
use crate::entity::r#type::from_type;
use crate::world::BlockFlags;
use crate::world::World;
use pumpkin_data::HorizontalFacingExt;
use pumpkin_data::block_properties::{BeeNestLikeProperties, BlockProperties};
use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

const MIN_OCCUPATION_TICKS_NECTAR: i32 = 2_400;
const MIN_OCCUPATION_TICKS_NECTARLESS: i32 = 600;

pub struct BeehiveBlockEntity {
    pub position: BlockPos,
    pub bees: Mutex<Option<Vec<NbtTag>>>,
    pub flower_pos: Mutex<Option<BlockPos>>,
}

fn occupant_entity_data(tag: &NbtTag) -> Option<NbtCompound> {
    let compound = tag.extract_compound()?;
    Some(
        compound
            .get_compound("entity_data")
            .cloned()
            .unwrap_or_else(|| compound.clone()),
    )
}

impl BlockEntity for BeehiveBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        // 26.2 stores typed occupants under the lowercase `bees` key.  Keep
        // accepting the legacy pre-26 uppercase form so existing worlds do
        // not lose their inhabitants on upgrade.
        let bees = nbt
            .get_list("bees")
            .or_else(|| nbt.get_list("Bees"))
            .map(<[_]>::to_vec);
        let flower_pos = nbt.get_compound("FlowerPos").map(|c| {
            BlockPos::new(
                c.get_int("X").unwrap_or(0),
                c.get_int("Y").unwrap_or(0),
                c.get_int("Z").unwrap_or(0),
            )
        });
        Self {
            position,
            bees: Mutex::new(bees),
            flower_pos: Mutex::new(flower_pos),
        }
    }

    fn write_nbt<'a>(
        &'a self,
        nbt: &'a mut NbtCompound,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            if let Some(b) = self.bees.lock().await.as_ref() {
                nbt.put_list("bees", b.clone());
            }
            if let Some(fp) = self.flower_pos.lock().await.as_ref() {
                let mut fp_nbt = NbtCompound::new();
                fp_nbt.put_int("X", fp.0.x);
                fp_nbt.put_int("Y", fp.0.y);
                fp_nbt.put_int("Z", fp.0.z);
                nbt.put_compound("FlowerPos", fp_nbt);
            }
        })
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        if let Ok(bees) = self.bees.try_lock()
            && let Some(ref b) = *bees
        {
            nbt.put_list("bees", b.clone());
        }
        if let Ok(flower_pos) = self.flower_pos.try_lock()
            && let Some(ref fp) = *flower_pos
        {
            let mut fp_nbt = NbtCompound::new();
            fp_nbt.put_int("X", fp.0.x);
            fp_nbt.put_int("Y", fp.0.y);
            fp_nbt.put_int("Z", fp.0.z);
            nbt.put_compound("FlowerPos", fp_nbt);
        }
        Some(nbt)
    }

    fn tick<'a>(&'a self, world: &'a Arc<World>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            self.tick_occupants(world).await;

            // Vanilla emits a low-volume work sound independently of the
            // release timer while a hive still contains bees.
            if self.has_occupants().await && rand::random_bool(0.005) {
                world.play_block_sound(
                    Sound::BlockBeehiveWork,
                    SoundCategory::Blocks,
                    self.position,
                );
            }
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl BeehiveBlockEntity {
    pub const ID: &'static str = "minecraft:beehive";
    #[must_use]
    pub const fn new(position: BlockPos) -> Self {
        Self {
            position,
            bees: Mutex::const_new(None),
            flower_pos: Mutex::const_new(None),
        }
    }

    async fn has_occupants(&self) -> bool {
        self.bees
            .lock()
            .await
            .as_ref()
            .is_some_and(|bees| !bees.is_empty())
    }

    /// Advances the persisted vanilla `BeeData` counters without throwing
    /// away unknown fields.  `BeeData.tick()` releases only when the previous
    /// value is strictly greater than `min_ticks_in_hive`; reproducing that
    /// ordering matters for the 601/2401-tick boundaries.
    fn advance_occupant(tag: &mut NbtTag) -> bool {
        let Some(compound) = tag.extract_compound() else {
            return false;
        };
        let entity_data = occupant_entity_data(tag);
        let has_nectar = entity_data
            .as_ref()
            .and_then(|entity| entity.get_bool("HasNectar"))
            .unwrap_or(false);
        let ticks = compound.get_int("ticks_in_hive").unwrap_or(0);
        let min_ticks = compound
            .get_int("min_ticks_in_hive")
            .unwrap_or(if has_nectar {
                MIN_OCCUPATION_TICKS_NECTAR
            } else {
                MIN_OCCUPATION_TICKS_NECTARLESS
            });

        // NbtTag has no mutable extraction helper; update the original
        // compound through the enum variant while retaining every unrelated
        // future-version field.
        if let NbtTag::Compound(compound) = tag {
            compound.put_int("ticks_in_hive", ticks.saturating_add(1));
            if !compound.has("min_ticks_in_hive") {
                compound.put_int("min_ticks_in_hive", min_ticks);
            }
        }
        ticks > min_ticks
    }

    /// Runs the server-side hive occupant timer and releases bees once the
    /// front face is clear.  A blocked exit keeps the occupant in the hive,
    /// exactly like `BeehiveBlockEntity.tickOccupants` in vanilla.
    async fn tick_occupants(&self, world: &Arc<World>) {
        let Some(stored) = self.bees.lock().await.take() else {
            return;
        };

        let facing = world.get_block_and_state(&self.position).1;
        let block = world.get_block(&self.position);
        let facing = if BeeNestLikeProperties::handles_block_id(block.id) {
            BeeNestLikeProperties::from_state_id(facing.id, block).facing
        } else {
            // A broken/unknown block state must not make occupants disappear;
            // use the canonical south-facing default for the retry path.
            pumpkin_data::block_properties::HorizontalFacing::South
        };

        let mut remaining = Vec::with_capacity(stored.len());
        for mut tag in stored {
            let ready = Self::advance_occupant(&mut tag);
            if !ready || !self.release_one(world, &tag, facing, false, true).await {
                remaining.push(tag);
            }
        }

        *self.bees.lock().await = (!remaining.is_empty()).then_some(remaining);
    }

    /// Spawns one stored bee if its exit is unobstructed.  `emergency` is
    /// reserved for future fire-nearby handling; normal and shears releases
    /// both require the front collision shape to be empty in vanilla.
    /// `deliver_nectar` distinguishes a normal `HONEY_DELIVERED` release from
    /// `BEE_RELEASED` (breaking/shearing or bottle collection): only the
    /// former increments the hive's honey level.
    async fn release_one(
        &self,
        world: &Arc<World>,
        tag: &NbtTag,
        facing: pumpkin_data::block_properties::HorizontalFacing,
        emergency: bool,
        deliver_nectar: bool,
    ) -> bool {
        let Some(mut entity_data) = occupant_entity_data(tag) else {
            return false;
        };
        let Some(id) = entity_data.get_string("id") else {
            return false;
        };
        let Some(entity_type) = EntityType::from_name(id.strip_prefix("minecraft:").unwrap_or(id))
        else {
            return false;
        };
        if entity_type.id != EntityType::BEE.id {
            return false;
        }

        let front = self
            .position
            .offset(facing.to_block_direction().to_offset());
        let front_blocked = !world
            .get_block_state(&front)
            .get_block_collision_shapes()
            .next()
            .is_none();
        if front_blocked && !emergency {
            return false;
        }

        // Vanilla restores the hive's remembered flower when the bee does not
        // already have one.  Keep the same NBT key used by Bee.java.
        if !entity_data.has("flower_pos")
            && let Some(flower_pos) = *self.flower_pos.lock().await
        {
            let mut flower = NbtCompound::new();
            flower.put_int("X", flower_pos.0.x);
            flower.put_int("Y", flower_pos.0.y);
            flower.put_int("Z", flower_pos.0.z);
            entity_data.put_compound("flower_pos", flower);
        }

        let uuid = entity_data.get_uuid("UUID").unwrap_or_else(Uuid::new_v4);
        let entity = from_type(entity_type, self.position.to_centered_f64(), world, uuid);
        entity.read_nbt_non_mut(&entity_data).await;

        let direction = facing.to_block_direction().to_offset();
        let delta = 0.55 + f64::from(entity.get_entity().width()) / 2.0;
        let center = self.position.to_centered_f64();
        let spawn_position = Vector3::new(
            center.x + delta * f64::from(direction.x),
            center.y - f64::from(entity.get_entity().height()) / 2.0,
            center.z + delta * f64::from(direction.z),
        );
        entity.get_entity().set_pos(spawn_position);
        entity.get_entity().velocity.store(Vector3::default());
        world.spawn_entity(entity).await;
        world.play_block_sound(
            Sound::BlockBeehiveExit,
            SoundCategory::Blocks,
            self.position,
        );

        // A bee leaving with nectar increases honey by 1 (occasionally 2),
        // capped at level five.  This is the server-side source of the honey
        // level change; bottles/shears only read the resulting block state.
        if deliver_nectar && entity_data.get_bool("HasNectar").unwrap_or(false) {
            let (block, state) = world.get_block_and_state(&self.position);
            if BeeNestLikeProperties::handles_block_id(block.id) {
                let mut props = BeeNestLikeProperties::from_state_id(state.id, block);
                if props.honey_level < 5 {
                    let increase = if rand::random_range(0..100) == 0 {
                        2
                    } else {
                        1
                    };
                    props.honey_level = (props.honey_level + increase).min(5);
                    world
                        .clone()
                        .set_block_state(
                            &self.position,
                            props.to_state_id(block),
                            BlockFlags::NOTIFY_ALL,
                        )
                        .await;
                }
            }
        }
        true
    }

    /// Releases stored bees when the hive is opened by shears or otherwise
    /// broken.  Modern worlds store each occupant as
    /// `{entity_data: {id: "minecraft:bee", ...}, ticks_in_hive, ...}`;
    /// older worlds may store the entity compound directly.  Unknown or
    /// malformed entries are retained so a future Pumpkin version can still
    /// restore them instead of silently deleting inhabitants.
    pub async fn release_occupants(
        &self,
        world: &Arc<World>,
        facing: pumpkin_data::block_properties::HorizontalFacing,
    ) {
        let Some(stored) = self.bees.lock().await.take() else {
            return;
        };

        let mut remaining = Vec::new();

        for tag in stored {
            if !self.release_one(world, &tag, facing, false, false).await {
                remaining.push(tag);
            }
        }

        *self.bees.lock().await = (!remaining.is_empty()).then_some(remaining);
    }
}

#[cfg(test)]
mod tests {
    use super::occupant_entity_data;
    use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

    #[test]
    fn occupant_data_accepts_modern_wrapped_and_legacy_direct_forms() {
        let mut entity = NbtCompound::new();
        entity.put_string("id", "minecraft:bee".to_string());

        let mut modern = NbtCompound::new();
        modern.put_compound("entity_data", entity.clone());
        let modern = occupant_entity_data(&NbtTag::Compound(modern)).expect("modern occupant");
        assert_eq!(modern.get_string("id"), Some("minecraft:bee"));

        let legacy = occupant_entity_data(&NbtTag::Compound(entity)).expect("legacy occupant");
        assert_eq!(legacy.get_string("id"), Some("minecraft:bee"));
    }

    #[test]
    fn malformed_occupant_is_not_released() {
        assert!(occupant_entity_data(&NbtTag::String("not-a-compound".into())).is_none());
        let mut wrapper = NbtCompound::new();
        wrapper.put_compound("entity_data", NbtCompound::new());
        assert_eq!(
            occupant_entity_data(&NbtTag::Compound(wrapper)),
            Some(NbtCompound::new())
        );
    }

    #[test]
    fn occupant_timer_uses_vanilla_strict_boundary() {
        let mut entity = NbtCompound::new();
        entity.put_string("id", "minecraft:bee".to_string());
        let mut occupant = NbtCompound::new();
        occupant.put_compound("entity_data", entity);
        occupant.put_int("ticks_in_hive", 600);
        occupant.put_int("min_ticks_in_hive", 600);
        let mut tag = NbtTag::Compound(occupant);

        assert!(!super::BeehiveBlockEntity::advance_occupant(&mut tag));
        assert_eq!(
            tag.extract_compound()
                .and_then(|c| c.get_int("ticks_in_hive")),
            Some(601)
        );
        assert!(super::BeehiveBlockEntity::advance_occupant(&mut tag));
        assert_eq!(
            tag.extract_compound()
                .and_then(|c| c.get_int("ticks_in_hive")),
            Some(602)
        );
    }

    #[test]
    fn occupant_timer_defaults_to_nectar_duration() {
        let mut entity = NbtCompound::new();
        entity.put_string("id", "minecraft:bee".to_string());
        entity.put_bool("HasNectar", true);
        let mut occupant = NbtCompound::new();
        occupant.put_compound("entity_data", entity);
        occupant.put_int("ticks_in_hive", 2_400);
        let mut tag = NbtTag::Compound(occupant);

        assert!(!super::BeehiveBlockEntity::advance_occupant(&mut tag));
        assert_eq!(
            tag.extract_compound()
                .and_then(|c| c.get_int("min_ticks_in_hive")),
            Some(2_400)
        );
        assert!(super::BeehiveBlockEntity::advance_occupant(&mut tag));
    }
}
