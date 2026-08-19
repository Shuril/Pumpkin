use pumpkin_data::{Block, BlockId, BlockState};

use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::random::{RandomGenerator, get_seed, xoroshiro128::Xoroshiro};

use crate::entity::experience_orb::ExperienceOrbEntity;
use crate::entity::player::Player;
use crate::world::World;
use crate::world::loot::{LootContextParameters, LootTableExt};
use std::pin::Pin;
use std::sync::Arc;

pub mod blocks;
pub mod entities;
pub mod fluid;
pub mod registry;
pub mod viewer;

use crate::block::registry::BlockActionResult;
use crate::entity::EntityBase;
use crate::entity::projectile::ProjectileHit;
use crate::server::Server;
use pumpkin_data::BlockDirection;
use pumpkin_data::block_rotation::{Mirror, Rotation};
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_protocol::java::server::play::SUseItemOn;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::{BlockAccessor, BlockFlags};

pub trait BlockMetadata {
    fn ids() -> Box<[BlockId]>;
}

pub trait FluidMetadata {
    fn ids() -> Box<[u16]>;
}

pub type BlockFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub(crate) fn stop_vertical_movement_after_fall(entity: &dyn EntityBase) {
    let entity = entity.get_entity();
    let mut velocity = entity.velocity.load();
    velocity.y = 0.0;
    entity.velocity.store(velocity);
}

pub(crate) fn bounce_entity_after_fall(entity: &dyn EntityBase, bounce_multiplier: f64) {
    let base_entity = entity.get_entity();
    let mut velocity = base_entity.velocity.load();

    if base_entity.is_sneaking() {
        velocity.y = 0.0;
    } else if velocity.y < 0.0 {
        let entity_factor = if entity.get_living_entity().is_some() {
            1.0
        } else {
            0.8
        };
        velocity.y = -velocity.y * bounce_multiplier * entity_factor;
    }

    base_entity.velocity.store(velocity);
}

pub trait BlockBehaviour: Send + Sync {
    fn normal_use<'a>(&'a self, _args: NormalUseArgs<'a>) -> BlockFuture<'a, BlockActionResult> {
        Box::pin(async move { BlockActionResult::Pass })
    }

    fn use_with_item<'a>(
        &'a self,
        _args: UseWithItemArgs<'a>,
    ) -> BlockFuture<'a, BlockActionResult> {
        Box::pin(async move { BlockActionResult::PassToDefaultBlockAction })
    }

    fn on_entity_collision<'a>(&'a self, _args: OnEntityCollisionArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Called when an entity is standing on / walking over the top face of this block.
    fn on_entity_step<'a>(&'a self, _args: OnEntityStepArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async {})
    }

    fn should_drop_items_on_explosion(&self) -> bool {
        true
    }

    fn explode<'a>(&'a self, _args: ExplodeArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Handles the block event, which is an event specific to a block with an integer ID and data.
    ///
    /// returns whether the event was handled successfully
    fn on_synced_block_event<'a>(
        &'a self,
        _args: OnSyncedBlockEventArgs<'a>,
    ) -> BlockFuture<'a, bool> {
        Box::pin(async move { false })
    }

    /// getPlacementState in source code
    fn on_place<'a>(&'a self, args: OnPlaceArgs<'a>) -> BlockFuture<'a, BlockStateId> {
        Box::pin(async move { args.block.default_state.id })
    }

    fn random_tick<'a>(&'a self, _args: RandomTickArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async {})
    }

    fn can_place_at(&self, _args: CanPlaceAtArgs<'_>) -> bool {
        true
    }

    fn can_update_at(&self, _args: CanUpdateAtArgs<'_>) -> bool {
        false
    }

    /// onBlockAdded in source code
    fn placed<'a>(&'a self, _args: PlacedArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async {})
    }

    fn player_placed<'a>(&'a self, _args: PlayerPlacedArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async {})
    }

    fn on_landed_upon<'a>(&'a self, args: OnLandedUponArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            if let Some(living) = args.entity.get_living_entity() {
                living
                    .handle_fall_damage(args.entity, args.fall_distance, 1.0)
                    .await;
            }
        })
    }

    fn update_entity_movement_after_fall_on<'a>(
        &'a self,
        args: UpdateEntityMovementAfterFallOnArgs<'a>,
    ) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            stop_vertical_movement_after_fall(args.entity);
        })
    }

    fn broken<'a>(&'a self, _args: BrokenArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async {})
    }

    fn on_neighbor_update<'a>(&'a self, _args: OnNeighborUpdateArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Called if a block state is replaced or it replaces another state
    fn prepare<'a>(&'a self, _args: PrepareArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async {})
    }

    fn get_state_for_neighbor_update<'a>(
        &'a self,
        args: GetStateForNeighborUpdateArgs<'a>,
    ) -> BlockFuture<'a, BlockStateId> {
        Box::pin(async move { args.state_id })
    }

    fn on_scheduled_tick<'a>(&'a self, _args: OnScheduledTickArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Handles a projectile impacting this block.  This is the block-side
    /// equivalent of vanilla `Block#onProjectileHit`; the world dispatches it
    /// before the projectile's own hit callback so a block can update its
    /// state (for example, a target block's redstone power) while the exact
    /// impact coordinates are still available.
    fn on_projectile_hit<'a>(&'a self, _args: OnProjectileHitArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async {})
    }

    fn on_state_replaced<'a>(&'a self, _args: OnStateReplacedArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async {})
    }

    // --- Redstone/Comparator Methods ---

    /// Sides where redstone connects to
    fn emits_redstone_power<'a>(
        &'a self,
        _args: EmitsRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, bool> {
        Box::pin(async move { false })
    }

    /// Weak redstone power, aka. block that should be powered needs to be directly next to the source block
    fn get_weak_redstone_power<'a>(
        &'a self,
        _args: GetRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, u8> {
        Box::pin(async move { 0 })
    }

    /// Strong redstone power. this can power a block that then gives power
    fn get_strong_redstone_power<'a>(
        &'a self,
        _args: GetRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, u8> {
        Box::pin(async move { 0 })
    }

    fn get_comparator_output<'a>(
        &'a self,
        _args: GetComparatorOutputArgs<'a>,
    ) -> BlockFuture<'a, Option<u8>> {
        Box::pin(async move { None })
    }

    fn get_inside_collision_shape<'a>(
        &'a self,
        _args: GetInsideCollisionShapeArgs<'a>,
    ) -> BlockFuture<'a, BoundingBox> {
        Box::pin(async move { BoundingBox::full_block() })
    }

    fn mirror(&self, block: &Block, state_id: BlockStateId, mirror: Mirror) -> &'static BlockState {
        block.mirror(state_id, mirror)
    }

    fn rotate(
        &self,
        block: &Block,
        state_id: BlockStateId,
        rotation: Rotation,
    ) -> &'static BlockState {
        block.rotate(state_id, rotation)
    }
}

pub struct NormalUseArgs<'a> {
    pub server: &'a Server,
    pub world: &'a Arc<World>,
    pub block: &'a Block,
    pub position: &'a BlockPos,
    pub player: &'a Arc<Player>,
    pub hit: &'a BlockHitResult<'a>,
}

pub struct UseWithItemArgs<'a> {
    pub server: &'a Server,
    pub world: &'a Arc<World>,
    pub block: &'a Block,
    pub position: &'a BlockPos,
    pub player: &'a Arc<Player>,
    pub hit: &'a BlockHitResult<'a>,
    pub item_stack: &'a mut ItemStack,
    pub equipment_slot: &'a EquipmentSlot,
}

pub struct BlockHitResult<'a> {
    pub face: &'a BlockDirection,
    pub cursor_pos: &'a Vector3<f32>,
}

pub struct OnEntityCollisionArgs<'a> {
    pub server: &'a Server,
    pub world: &'a Arc<World>,
    pub block: &'a Block,
    pub state: &'a BlockState,
    pub position: &'a BlockPos,
    pub entity: &'a dyn EntityBase,
}

pub struct OnEntityStepArgs<'a> {
    pub world: &'a Arc<World>,
    pub block: &'a Block,
    pub state: &'a BlockState,
    pub position: &'a BlockPos,
    pub entity: &'a dyn EntityBase,
    pub below_supporting_block: bool,
}

pub struct ExplodeArgs<'a> {
    pub world: &'a Arc<World>,
    pub block: &'a Block,
    pub position: &'a BlockPos,
}

pub struct OnSyncedBlockEventArgs<'a> {
    pub world: &'a Arc<World>,
    pub block: &'a Block,
    pub position: &'a BlockPos,
    pub r#type: u8,
    pub data: u8,
}

pub struct OnPlaceArgs<'a> {
    pub server: &'a Server,
    pub world: &'a World,
    pub block: &'a Block,
    pub position: &'a BlockPos,
    pub direction: BlockDirection,
    pub player: &'a Player,
    pub replacing: BlockIsReplacing,
    pub use_item_on: &'a SUseItemOn,
}

pub struct RandomTickArgs<'a> {
    pub world: &'a Arc<World>,
    pub block: &'a Block,
    pub position: &'a BlockPos,
}

pub struct CanPlaceAtArgs<'a> {
    pub server: Option<&'a Server>,
    pub world: Option<&'a World>,
    pub block_accessor: &'a dyn BlockAccessor,
    pub block: &'a Block,
    pub state: &'a BlockState,
    pub position: &'a BlockPos,
    pub direction: Option<BlockDirection>,
    pub player: Option<&'a Player>,
    pub use_item_on: Option<&'a SUseItemOn>,
}

pub struct CanUpdateAtArgs<'a> {
    pub world: &'a World,
    pub block: &'a Block,
    pub state_id: BlockStateId,
    pub position: &'a BlockPos,
    pub direction: BlockDirection,
    pub player: &'a Player,
    pub use_item_on: &'a SUseItemOn,
}

pub struct PlacedArgs<'a> {
    pub world: &'a Arc<World>,
    pub block: &'a Block,
    pub state_id: BlockStateId,
    pub old_state_id: BlockStateId,
    pub position: &'a BlockPos,
    pub notify: bool,
}

pub struct PlayerPlacedArgs<'a> {
    pub world: &'a Arc<World>,
    pub block: &'a Block,
    pub state_id: BlockStateId,
    pub position: &'a BlockPos,
    pub direction: BlockDirection,
    pub player: &'a Player,
}

pub struct OnLandedUponArgs<'a> {
    pub world: &'a Arc<World>,
    pub fall_distance: f32,
    pub entity: &'a dyn EntityBase,
}

pub struct UpdateEntityMovementAfterFallOnArgs<'a> {
    pub entity: &'a dyn EntityBase,
}

pub struct BrokenArgs<'a> {
    pub block: &'a Block,
    pub player: &'a Arc<Player>,
    /// The tool used for the break.  Vanilla `spawnAfterBreak` handlers use
    /// the exact stack (not just the player's game mode) for enchantment-gated
    /// side effects such as infested-block silverfish spawning.
    pub tool: &'a ItemStack,
    pub position: &'a BlockPos,
    pub server: &'a Server,
    pub world: &'a Arc<World>,
    pub state: &'a BlockState,
}

pub struct OnNeighborUpdateArgs<'a> {
    pub world: &'a Arc<World>,
    pub block: &'a Block,
    pub position: &'a BlockPos,
    pub source_block: &'a Block,
    pub notify: bool,
}

pub struct PrepareArgs<'a> {
    pub world: &'a Arc<World>,
    pub block: &'a Block,
    pub state_id: BlockStateId,
    pub position: &'a BlockPos,
    pub flags: BlockFlags,
}

pub struct GetStateForNeighborUpdateArgs<'a> {
    pub world: &'a World,
    pub block: &'a Block,
    pub state_id: BlockStateId,
    pub position: &'a BlockPos,
    pub direction: BlockDirection,
    pub neighbor_position: &'a BlockPos,
    pub neighbor_state_id: BlockStateId,
}

pub struct OnScheduledTickArgs<'a> {
    pub world: &'a Arc<World>,
    pub block: &'a Block,
    pub position: &'a BlockPos,
}

pub struct OnProjectileHitArgs<'a> {
    pub world: &'a Arc<World>,
    pub block: &'a Block,
    pub state: &'a BlockState,
    pub position: &'a BlockPos,
    pub hit: &'a ProjectileHit,
    pub projectile: &'a dyn EntityBase,
}

pub struct OnStateReplacedArgs<'a> {
    pub world: &'a Arc<World>,
    pub block: &'a Block,
    pub old_state_id: BlockStateId,
    pub position: &'a BlockPos,
    pub moved: bool,
}

pub struct EmitsRedstonePowerArgs<'a> {
    pub block: &'a Block,
    pub state: &'a BlockState,
    pub direction: BlockDirection,
}

pub struct GetRedstonePowerArgs<'a> {
    pub world: &'a World,
    pub block: &'a Block,
    pub state: &'a BlockState,
    pub position: &'a BlockPos,
    pub direction: BlockDirection,
}

pub struct GetComparatorOutputArgs<'a> {
    pub world: &'a World,
    pub block: &'a Block,
    pub state: &'a BlockState,
    pub position: &'a BlockPos,
}

pub struct GetInsideCollisionShapeArgs<'a> {
    pub world: &'a World,
    pub block: &'a Block,
    pub state: &'a BlockState,
    pub position: &'a BlockPos,
}

#[derive(Clone)]
pub struct BlockEvent {
    pub pos: BlockPos,
    pub r#type: u8,
    pub data: u8,
}

pub async fn drop_loot(
    world: &Arc<World>,
    block: &Block,
    pos: &BlockPos,
    experience: bool,
    mut params: LootContextParameters,
) {
    // Vanilla's `doTileDrops` gamerule applies to the central loot-table path,
    // including explosions and blocks broken by non-player causes. Keeping
    // the guard here avoids duplicating it at those call sites and also
    // suppresses block experience, which is part of the same rule.
    if !world.level_info.load().game_rules.block_drops {
        return;
    }

    // Silk Touch suppresses block experience in vanilla, even when the block
    // itself has an experience range.  Capture this before consuming the loot
    // context below; all block-break and explosion callers share this gate.
    let silk_touch = params
        .tool
        .as_ref()
        .is_some_and(|tool| tool.get_enchantment_level(&pumpkin_data::Enchantment::SILK_TOUCH) > 0);
    // Keep the experience roll tied to the same authoritative operation as
    // the item loot.  Using `get_seed()` here made two replays of the same
    // block break disagree even when the loot context had an explicit seed,
    // and also allowed concurrent drops to perturb one another.  XP gets its
    // own fixed salt so it does not consume or alias the item-loot stream.
    let experience_seed = derive_experience_seed(params.random_seed);
    params.attach_biome_resolver(world);

    if let Some(loot_table) = &block.loot_table {
        for stack in loot_table.get_loot(params) {
            world.drop_stack(pos, stack).await;
        }
    }

    if block_experience_allowed(experience, silk_touch)
        && let Some(experience) = &block.experience
    {
        let mut random = RandomGenerator::Xoroshiro(Xoroshiro::from_seed(experience_seed));
        let amount = experience.experience.get(&mut random);
        if amount > 0 {
            ExperienceOrbEntity::spawn(world, pos.to_f64(), amount as u32).await;
        }
    }
}

/// Vanilla's block-break experience gate: the action must request experience
/// and Silk Touch must not be active on the tool.  Keeping this pure avoids
/// accidentally applying the rule only to player breaks while explosions and
/// other block-destruction paths use the same `drop_loot` function.
#[must_use]
pub const fn block_experience_allowed(requested: bool, silk_touch: bool) -> bool {
    requested && !silk_touch
}

/// Derives the independent RNG stream used for a block's experience amount.
/// `None` is retained for legacy/plugin callers that do not provide a loot
/// context seed; world-bound paths always pass `Some`.
#[must_use]
pub fn derive_experience_seed(seed: Option<u64>) -> u64 {
    seed.map_or_else(get_seed, |seed| seed ^ 0x4558_5045_5249_454eu64)
}

pub async fn calc_block_breaking(
    player: &Player,
    state: &BlockState,
    block: &'static Block,
) -> f32 {
    let hardness = state.hardness;
    #[expect(clippy::float_cmp)]
    if hardness == -1.0 {
        // unbreakable
        return 0.0;
    }
    let i = if player.can_harvest(state, block).await {
        30.0
    } else {
        100.0
    };

    player.get_mining_speed(block).await / hardness / i
}

#[derive(PartialEq, Eq, Debug)]
pub enum BlockIsReplacing {
    Itself(BlockStateId),
    Water(u8),
    Other,
    None,
}

impl BlockIsReplacing {
    #[must_use]
    /// Returns true if the block was a water source block.
    pub const fn water_source(&self) -> bool {
        match self {
            // Level 0 means the water is a source block
            Self::Water(level) => *level == 0,
            _ => false,
        }
    }
}

pub async fn calculate_comparator_output(
    inventory: &dyn pumpkin_world::inventory::Inventory,
) -> u8 {
    let size = inventory.size();
    if size == 0 {
        return 0;
    }
    let mut fill_sum = 0.0;
    let mut non_empty_count = 0;
    for i in 0..size {
        let stack = inventory.get_stack(i).await;
        if !stack.is_empty() {
            let max_stack = stack.get_max_stack_size() as f32;
            let count = stack.item_count as f32;
            fill_sum += count / max_stack;
            non_empty_count += 1;
        }
    }
    if non_empty_count == 0 {
        return 0;
    }
    let percentage = fill_sum / (size as f32);
    let output = 1.0 + percentage * 14.0;
    output.floor() as u8
}

#[cfg(test)]
mod tests {
    use super::{block_experience_allowed, derive_experience_seed};

    #[test]
    fn silk_touch_suppresses_block_experience_for_every_break_source() {
        assert!(block_experience_allowed(true, false));
        assert!(!block_experience_allowed(true, true));
        assert!(!block_experience_allowed(false, false));
    }

    #[test]
    fn block_experience_seed_is_stable_and_separate_from_loot_seed() {
        let first = derive_experience_seed(Some(0x1234));
        assert_eq!(first, derive_experience_seed(Some(0x1234)));
        assert_ne!(first, 0x1234);
        assert_ne!(first, derive_experience_seed(Some(0x1235)));
    }
}
