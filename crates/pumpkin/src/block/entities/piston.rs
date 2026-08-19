use std::sync::atomic::{AtomicI64, Ordering};
use std::{pin::Pin, sync::Arc};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::{Block, BlockDirection, BlockState, BlockStateId};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};

use crate::world::{BlockFlags, World};

use super::BlockEntity;

pub struct PistonBlockEntity {
    pub position: BlockPos,
    pub pushed_block_state: &'static BlockState,
    pub facing: BlockDirection,
    pub current_progress: AtomicCell<f32>,
    pub last_progress: AtomicCell<f32>,
    /// Game time at which vanilla's moving-piston tick last ran.
    ///
    /// `PistonBaseBlock` consults this value when a retraction is queued in
    /// the same game tick as the moving piston.  It is intentionally runtime
    /// state: Mojang's `lastTicked` field is not serialized in block-entity
    /// NBT, and a chunk reload must not make an old tick look current.
    pub last_ticked: AtomicI64,
    pub extending: bool,
    pub source: bool,
}

impl PistonBlockEntity {
    pub const ID: &'static str = "minecraft:piston";

    #[inline]
    #[must_use]
    pub(crate) fn should_use_final_tick(progress: f32, game_time: i64, last_ticked: i64) -> bool {
        progress < 0.5 || game_time == last_ticked
    }

    const fn movement_direction(&self) -> BlockDirection {
        if self.extending {
            self.facing
        } else {
            self.facing.opposite()
        }
    }

    /// Vanilla's `getAmountExtended`: how far back from the block's final position
    /// the visual is at a given animation progress. Negative for extending.
    fn amount_extended(&self, progress: f32) -> f32 {
        if self.extending {
            progress - 1.0
        } else {
            1.0 - progress
        }
    }

    fn dir_vec(dir: BlockDirection, scale: f64) -> Vector3<f64> {
        let off = dir.to_offset();
        Vector3::new(
            f64::from(off.x) * scale,
            f64::from(off.y) * scale,
            f64::from(off.z) * scale,
        )
    }

    /// Ports vanilla `PistonBlockEntity.pushEntities`: pushes entities whose
    /// bounding box intersects the moving block's swept volume during this tick.
    fn push_entities(&self, world: &Arc<World>, new_progress: f32) {
        let last = self.last_progress.load();
        let delta = f64::from(new_progress - last);
        if delta <= 0.0 {
            return;
        }

        let motion_dir = self.movement_direction();
        let amount = f64::from(self.amount_extended(last));

        // Block AABB at the current visual position (start of this tick).
        // For source=true (the piston head BE), vanilla uses the piston-head
        // collision shape — a 4-pixel-thick slab at the front face — rather than
        // the full cube. Without this, the head "sweeps" too much volume and
        // drags entities (e.g. end crystals) back with it during retraction.
        let raw_block_aabb = BoundingBox::from_block(&self.position);
        let block_aabb = if self.source {
            Self::head_shape_aabb(raw_block_aabb, self.facing)
        } else {
            raw_block_aabb
        }
        .shift(Self::dir_vec(self.facing, amount));

        // Stretch by motion to get the swept volume (one tick of animation).
        let motion = Self::dir_vec(motion_dir, delta);
        let swept = block_aabb.stretch(motion);

        for entity in world.get_entities_at_box(&swept) {
            let e = entity.get_entity();
            if e.no_clip.load(Ordering::Relaxed) {
                continue;
            }
            // Player movement is client-authoritative; vanilla still nudges them
            // via Entity.move(PISTON), but we skip them here to avoid teleport jank.
            if entity.get_player().is_some() {
                continue;
            }

            let entity_aabb = e.bounding_box.load();
            let intersection = Self::intersection_size(swept, motion_dir, entity_aabb);
            if intersection <= 0.0 {
                continue;
            }
            let push_amount = intersection.min(delta) + 0.01;
            Self::move_entity(e, motion_dir, push_amount);

            // For a retracting head, vanilla also shoves the entity OUT of the
            // piston body cube. Without this, entities get pulled into the
            // piston and "stick" to it (looks like a sticky-piston drag).
            // For a retract-head BE, `self.position` already IS the piston
            // block position (it replaces the piston during animation).
            if !self.extending && self.source {
                Self::push_out_of_piston_body(e, &self.position, motion_dir, delta);
            }
        }
    }

    /// Vanilla's honey-block conveyor behavior. Entities standing on top of a
    /// horizontally moving honey block travel with it even when they do not
    /// intersect the block's swept collision volume.
    fn move_stuck_entities(&self, world: &Arc<World>, new_progress: f32) {
        if Block::from_state_id(self.pushed_block_state.id) != &Block::HONEY_BLOCK {
            return;
        }
        let movement = self.movement_direction();
        if matches!(movement, BlockDirection::Up | BlockDirection::Down) {
            return;
        }

        let delta = f64::from(new_progress - self.last_progress.load());
        if delta <= 0.0 {
            return;
        }

        let pos = self.position;
        // Vanilla's moveStuckEntities builds the sticky AABB from the
        // progress at the beginning of this tick.  `new_progress` is only
        // the delta endpoint; using it here moves riders one half-step early
        // and makes a chain of honey pistons visibly diverge.
        let current_progress = self.last_progress.load();
        let mut sticky_box = BoundingBox::new(
            Vector3::new(
                f64::from(pos.0.x),
                f64::from(pos.0.y) + 1.0,
                f64::from(pos.0.z),
            ),
            Vector3::new(
                f64::from(pos.0.x) + 1.0,
                f64::from(pos.0.y) + 1.500_001,
                f64::from(pos.0.z) + 1.0,
            ),
        );
        sticky_box = sticky_box.shift(Self::dir_vec(
            self.facing,
            f64::from(self.amount_extended(current_progress)),
        ));

        for entity in world.get_entities_at_box(&sticky_box) {
            let base = entity.get_entity();
            if !entity.is_pushable() || !base.on_ground.load(Ordering::Relaxed) {
                continue;
            }
            let entity_pos = base.pos.load();
            let supported_by_honey = base.get_supporting_block_pos() == Some(pos)
                || (entity_pos.x >= sticky_box.min.x
                    && entity_pos.x <= sticky_box.max.x
                    && entity_pos.z >= sticky_box.min.z
                    && entity_pos.z <= sticky_box.max.z);
            if supported_by_honey {
                Self::move_entity(base, movement, delta);
            }
        }
    }

    /// Vanilla `getIntersectionSize`: how much `entity` overlaps `swept` along
    /// `motion_dir`. Positive means the entity is in the path of the moving block.
    fn intersection_size(
        swept: BoundingBox,
        motion_dir: BlockDirection,
        entity: BoundingBox,
    ) -> f64 {
        match motion_dir {
            BlockDirection::East => swept.max.x - entity.min.x,
            BlockDirection::West => entity.max.x - swept.min.x,
            BlockDirection::Up => swept.max.y - entity.min.y,
            BlockDirection::Down => entity.max.y - swept.min.y,
            BlockDirection::South => swept.max.z - entity.min.z,
            BlockDirection::North => entity.max.z - swept.min.z,
        }
    }

    fn move_entity(entity: &crate::entity::Entity, dir: BlockDirection, distance: f64) {
        let new_pos = entity.pos.load() + Self::dir_vec(dir, distance);
        entity.set_pos(new_pos);
        entity.send_pos();
    }

    /// Vanilla `push`: when a piston head retracts, shove entities that ended up
    /// inside the piston-body cube back out the opposite direction (slightly past
    /// the move they just got, so the net motion is essentially zero).
    fn push_out_of_piston_body(
        entity: &crate::entity::Entity,
        piston_pos: &BlockPos,
        motion_dir: BlockDirection,
        amount: f64,
    ) {
        let body_aabb = BoundingBox::from_block(piston_pos);
        let entity_aabb = entity.bounding_box.load();
        if !body_aabb.intersects(&entity_aabb) {
            return;
        }
        let back = motion_dir.opposite();
        let e = Self::intersection_size(body_aabb, back, entity_aabb) + 0.01;
        let f = Self::intersection_size(
            body_aabb,
            back,
            Self::aabb_intersection(body_aabb, entity_aabb),
        ) + 0.01;
        if (e - f).abs() < 0.01 {
            let distance = e.min(amount) + 0.01;
            Self::move_entity(entity, back, distance);
        }
    }

    /// Approximation of vanilla `PistonHeadBlock` collision shape: a 4-pixel
    /// (0.25 block) slab at the front face of the block in the facing direction.
    fn head_shape_aabb(block_aabb: BoundingBox, facing: BlockDirection) -> BoundingBox {
        const HEAD_THICKNESS: f64 = 0.25;
        let mut min = block_aabb.min;
        let mut max = block_aabb.max;
        match facing {
            BlockDirection::East => min.x = max.x - HEAD_THICKNESS,
            BlockDirection::West => max.x = min.x + HEAD_THICKNESS,
            BlockDirection::Up => min.y = max.y - HEAD_THICKNESS,
            BlockDirection::Down => max.y = min.y + HEAD_THICKNESS,
            BlockDirection::South => min.z = max.z - HEAD_THICKNESS,
            BlockDirection::North => max.z = min.z + HEAD_THICKNESS,
        }
        BoundingBox::new(min, max)
    }

    const fn aabb_intersection(a: BoundingBox, b: BoundingBox) -> BoundingBox {
        BoundingBox::new(
            Vector3::new(
                a.min.x.max(b.min.x),
                a.min.y.max(b.min.y),
                a.min.z.max(b.min.z),
            ),
            Vector3::new(
                a.max.x.min(b.max.x),
                a.max.y.min(b.max.y),
                a.max.z.min(b.max.z),
            ),
        )
    }

    pub async fn finish(&self, world: Arc<World>) {
        if self.last_progress.load() < 1.0 {
            let pos = self.position;
            world.remove_block_entity(&pos);
            if world.get_block(&pos) == &Block::MOVING_PISTON {
                let state = if self.source {
                    Block::AIR.default_state.id
                } else {
                    world
                        .clone()
                        .update_from_neighbor_shapes(self.pushed_block_state.id, &pos)
                        .await
                };
                world
                    .clone()
                    .set_block_state(&pos, state, BlockFlags::NOTIFY_ALL)
                    .await;
                world.update_neighbors(&pos, None).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PistonBlockEntity;
    use crossbeam::atomic::AtomicCell;
    use pumpkin_data::{Block, BlockDirection};
    use pumpkin_util::math::position::BlockPos;
    use std::sync::atomic::AtomicI64;

    #[test]
    fn same_tick_moving_piston_uses_final_retraction_event() {
        assert!(PistonBlockEntity::should_use_final_tick(0.75, 42, 42));
        assert!(PistonBlockEntity::should_use_final_tick(0.25, 42, 7));
        assert!(!PistonBlockEntity::should_use_final_tick(0.75, 42, 7));
    }

    #[test]
    fn honey_sticky_offset_uses_start_of_tick_progress() {
        let piston = PistonBlockEntity {
            position: BlockPos::new(0, 0, 0),
            pushed_block_state: Block::HONEY_BLOCK.default_state,
            facing: BlockDirection::East,
            current_progress: AtomicCell::new(0.5),
            last_progress: AtomicCell::new(0.0),
            last_ticked: AtomicI64::new(-1),
            extending: true,
            source: false,
        };
        let start_offset = piston.amount_extended(piston.last_progress.load());
        let end_offset = piston.amount_extended(piston.current_progress.load());
        assert_eq!(start_offset, -1.0);
        assert_eq!(end_offset, -0.5);
        assert_ne!(start_offset, end_offset);
    }
}

const FACING: &str = "facing";
const LAST_PROGRESS: &str = "progress";
const EXTENDING: &str = "extending";
const SOURCE: &str = "source";
const PUSHED_BLOCK_STATE: &str = "blockState";

impl BlockEntity for PistonBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn tick<'a>(&'a self, world: &'a Arc<World>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            // Matches PistonMovingBlockEntity.tick: this is set before the
            // progress update so a nested neighbour update in the same world
            // tick can observe the exact game-time value.
            self.last_ticked
                .store(world.get_world_age().await, Ordering::Relaxed);
            let current_progress = self.current_progress.load();
            self.last_progress.store(current_progress);
            if current_progress >= 1.0 {
                let pos = self.position;
                world.remove_block_entity(&pos);
                if world.get_block(&pos) == &Block::MOVING_PISTON {
                    if self.pushed_block_state.is_air() {
                        world
                            .clone()
                            .set_block_state(
                                &pos,
                                self.pushed_block_state.id,
                                BlockFlags::FORCE_STATE | BlockFlags::MOVED,
                            )
                            .await;
                    } else {
                        let updated_state = world
                            .clone()
                            .update_from_neighbor_shapes(self.pushed_block_state.id, &pos)
                            .await;
                        world
                            .clone()
                            .set_block_state(
                                &pos,
                                updated_state,
                                BlockFlags::NOTIFY_ALL | BlockFlags::MOVED,
                            )
                            .await;
                        world.clone().update_neighbors(&pos, None).await;
                    }
                }
                return;
            }
            let new_progress = (current_progress + 0.5).min(1.0);
            self.push_entities(world, new_progress);
            self.move_stuck_entities(world, new_progress);
            self.current_progress.store(new_progress);
        })
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        // Pumpkin persists the validated global state id. This keeps moving
        // blocks intact across chunk unload/reload instead of turning them into
        // air. Invalid or older data safely falls back to air.
        let pushed_block_state = nbt
            .get_int(PUSHED_BLOCK_STATE)
            .and_then(|id| u16::try_from(id).ok())
            .and_then(BlockStateId::new)
            .map_or(Block::AIR.default_state, BlockState::from_id);
        let facing = nbt.get_byte(FACING).unwrap_or(0);
        let last_progress = nbt.get_float(LAST_PROGRESS).unwrap_or(0.0);
        let extending = nbt.get_bool(EXTENDING).unwrap_or(false);
        let source = nbt.get_bool(SOURCE).unwrap_or(false);
        Self {
            pushed_block_state,
            position,
            facing: BlockDirection::from_index(facing as u8).unwrap_or(BlockDirection::Down),
            current_progress: last_progress.into(),
            last_progress: last_progress.into(),
            last_ticked: AtomicI64::new(-1),
            extending,
            source,
        }
    }

    fn write_nbt<'a>(
        &'a self,
        nbt: &'a mut NbtCompound,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            nbt.put_int(
                PUSHED_BLOCK_STATE,
                i32::from(self.pushed_block_state.id.as_u16()),
            );
            nbt.put_byte(FACING, self.facing.to_index() as i8);
            nbt.put_float(LAST_PROGRESS, self.last_progress.load());
            nbt.put_bool(EXTENDING, self.extending);
            nbt.put_bool(SOURCE, self.source);
        })
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        nbt.put_int(
            PUSHED_BLOCK_STATE,
            i32::from(self.pushed_block_state.id.as_u16()),
        );
        nbt.put_byte(FACING, self.facing.to_index() as i8);
        nbt.put_float(LAST_PROGRESS, self.last_progress.load());
        nbt.put_bool(EXTENDING, self.extending);
        nbt.put_bool(SOURCE, self.source);
        // TODO: duplicated code because of async :c
        Some(nbt)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
