use std::{pin::Pin, sync::Arc};

use crate::{
    entity::player::Player,
    entity::r#type::from_type,
    item::{ItemBehaviour, ItemMetadata},
};
use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    data_component_impl::BucketEntityDataImpl,
    dimension::Dimension,
    entity::EntityType,
    fluid::Fluid,
    item::Item,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
};
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector3::Vector3},
};
use pumpkin_world::{tick::TickPriority, world::BlockFlags};

use crate::world::World;

pub struct EmptyBucketItem;
pub struct FilledBucketItem;
pub struct MilkBucketItem;

/// Returns the mob represented by a vanilla mob bucket. Water-bearing buckets
/// place their fluid first; the sulfur-cube bucket intentionally carries
/// `Fluids.EMPTY` and only spawns the entity, matching `MobBucketItem`.
pub(crate) fn bucket_entity_type(item: &Item) -> Option<&'static EntityType> {
    match item.id {
        id if id == Item::AXOLOTL_BUCKET.id => Some(&EntityType::AXOLOTL),
        id if id == Item::COD_BUCKET.id => Some(&EntityType::COD),
        id if id == Item::SALMON_BUCKET.id => Some(&EntityType::SALMON),
        id if id == Item::TROPICAL_FISH_BUCKET.id => Some(&EntityType::TROPICAL_FISH),
        id if id == Item::PUFFERFISH_BUCKET.id => Some(&EntityType::PUFFERFISH),
        id if id == Item::TADPOLE_BUCKET.id => Some(&EntityType::TADPOLE),
        id if id == Item::SULFUR_CUBE_BUCKET.id => Some(&EntityType::SULFUR_CUBE),
        _ => None,
    }
}

pub(crate) async fn spawn_bucket_entity(world: &Arc<World>, stack: &ItemStack, position: BlockPos) {
    let Some(entity_type) = bucket_entity_type(stack.item) else {
        return;
    };
    let entity = from_type(
        entity_type,
        position.to_f64().add_raw(0.5, 0.0, 0.5),
        world,
        uuid::Uuid::new_v4(),
    );
    crate::item::items::spawn_egg::apply_entity_variant(stack, entity.as_ref());
    if let Some(data) = stack.get_data_component::<BucketEntityDataImpl>()
        && let BucketEntityDataImpl::Compound(data) = data
    {
        // Bucket entity data is deliberately limited to the entity's custom
        // payload. Position/UUID/type are created by the server and must not
        // be overwritten by item NBT.
        let mut safe_data = data.clone();
        for key in ["Pos", "Motion", "Rotation", "UUID", "id"] {
            safe_data.child_tags.remove(key);
        }
        entity.read_nbt_non_mut(&safe_data).await;

        let base = entity.get_entity();
        if let Some(value) = safe_data.get_bool("NoGravity") {
            base.set_has_no_gravity(value);
        }
        if let Some(value) = safe_data.get_bool("Silent") {
            base.set_silent(value);
        }
        if let Some(value) = safe_data.get_bool("Glowing") {
            base.set_glowing(value).await;
        }
        if let Some(value) = safe_data.get_bool("NoAI")
            && let Some(mob) = entity.get_mob()
        {
            mob.get_mob_entity().set_no_ai(value);
        }
        if let Some(value) = safe_data.get_float("Health")
            && let Some(living) = entity.get_living_entity()
        {
            living.set_health(value);
        }
    }
    world.spawn_entity(entity).await;
}

impl ItemMetadata for EmptyBucketItem {
    fn ids() -> Box<[u16]> {
        [Item::BUCKET.id].into()
    }
}

impl ItemMetadata for FilledBucketItem {
    fn ids() -> Box<[u16]> {
        [
            Item::WATER_BUCKET.id,
            Item::LAVA_BUCKET.id,
            Item::POWDER_SNOW_BUCKET.id,
            Item::AXOLOTL_BUCKET.id,
            Item::COD_BUCKET.id,
            Item::SALMON_BUCKET.id,
            Item::TROPICAL_FISH_BUCKET.id,
            Item::PUFFERFISH_BUCKET.id,
            Item::TADPOLE_BUCKET.id,
            Item::SULFUR_CUBE_BUCKET.id,
        ]
        .into()
    }
}

impl ItemMetadata for MilkBucketItem {
    fn ids() -> Box<[u16]> {
        [Item::MILK_BUCKET.id].into()
    }
}

fn get_start_and_end_pos(player: &Player) -> (Vector3<f64>, Vector3<f64>) {
    let start_pos = player.eye_position();
    let (yaw, pitch) = player.rotation();
    let (yaw_rad, pitch_rad) = (f64::from(yaw.to_radians()), f64::from(pitch.to_radians()));
    let block_interaction_range = 4.5; // This is not the same as the block_interaction_range in the
    // player entity.
    let direction = Vector3::new(
        -yaw_rad.sin() * pitch_rad.cos() * block_interaction_range,
        -pitch_rad.sin() * block_interaction_range,
        pitch_rad.cos() * yaw_rad.cos() * block_interaction_range,
    );

    let end_pos = start_pos.add(&direction);
    (start_pos, end_pos)
}

fn waterlogged_check(block: &Block, state: BlockStateId) -> Option<bool> {
    block.properties(state).and_then(|properties| {
        properties
            .to_props()
            .into_iter()
            .find(|p| p.0 == "waterlogged")
            .map(|(_, value)| value == "true")
    })
}

fn is_waterlogged(block: &Block, state: BlockStateId) -> bool {
    waterlogged_check(block, state).unwrap_or(false)
}

fn set_waterlogged(block: &Block, state: BlockStateId, waterlogged: bool) -> BlockStateId {
    let Some(props) = block.properties(state) else {
        return state;
    };
    let original_props = &props.to_props();
    let waterlogged = waterlogged.to_string();
    let props: Vec<(&str, &str)> = original_props
        .iter()
        .map(|(key, value)| {
            if *key == "waterlogged" {
                ("waterlogged", waterlogged.as_str())
            } else {
                (*key, *value)
            }
        })
        .collect();
    block.from_properties(&props).to_state_id(block)
}

async fn give_player_bucket_item(player: &Player, item: &'static Item) {
    if player.gamemode.load() == GameMode::Creative {
        let inv = player.inventory.main_inventory.read().await;
        for stack in inv.iter() {
            if stack.item.id == item.id {
                return;
            }
        }
        drop(inv);
        let mut item_stack = ItemStack::new(1, item);
        player
            .inventory
            .insert_stack_anywhere(&mut item_stack)
            .await;
    } else {
        let item_stack = ItemStack::new(1, item);
        let mut held_stack = player.inventory.held_item().await;

        if held_stack.item_count == 1 {
            player.inventory.set_held_item(item_stack).await;
        } else {
            held_stack.decrement(1);
            player.inventory.set_held_item(held_stack).await;
            player
                .inventory
                .offer_or_drop_stack(item_stack, player)
                .await;
        }
    }
}

/// Tries to pick up powder snow, a waterlogged block, or a fluid source block at `block_pos`,
/// returning the matching filled bucket item on success.
pub(crate) async fn try_pickup_fluid_at(
    world: &Arc<World>,
    block_pos: BlockPos,
) -> Option<&'static Item> {
    let (block, state) = world.get_block_and_state_id(&block_pos);

    if block == &Block::POWDER_SNOW {
        world
            .break_block(
                &block_pos,
                None,
                BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
            )
            .await;
        return Some(&Item::POWDER_SNOW_BUCKET);
    }

    if is_waterlogged(block, state) {
        let state_id = set_waterlogged(block, state, false);
        world
            .set_block_state(&block_pos, state_id, BlockFlags::NOTIFY_NEIGHBORS)
            .await;
        world.schedule_fluid_tick(&Fluid::WATER, block_pos, 5, TickPriority::Normal);
        return Some(&Item::WATER_BUCKET);
    }

    if state == Block::LAVA.default_state.id || state == Block::WATER.default_state.id {
        world
            .break_block(&block_pos, None, BlockFlags::NOTIFY_NEIGHBORS)
            .await;
        world
            .set_block_state(
                &block_pos,
                Block::AIR.default_state.id,
                BlockFlags::NOTIFY_NEIGHBORS,
            )
            .await;
        return Some(if state == Block::LAVA.default_state.id {
            &Item::LAVA_BUCKET
        } else {
            &Item::WATER_BUCKET
        });
    }

    None
}

async fn try_pickup_bucket_item(
    world: &Arc<World>,
    block_pos: BlockPos,
    direction: BlockDirection,
) -> Option<&'static Item> {
    if let Some(item) = try_pickup_fluid_at(world, block_pos).await {
        return Some(item);
    }

    let target_pos = block_pos.offset(direction.to_offset());
    let (block, state) = world.get_block_and_state_id(&target_pos);
    if waterlogged_check(block, state).is_some() {
        let state_id = set_waterlogged(block, state, false);
        world
            .set_block_state(&target_pos, state_id, BlockFlags::NOTIFY_NEIGHBORS)
            .await;
        world.schedule_fluid_tick(&Fluid::WATER, target_pos, 5, TickPriority::Normal);
        return Some(&Item::WATER_BUCKET);
    }

    None
}

pub(crate) fn should_evaporate_in_nether(item: &Item, world: &World) -> bool {
    item.id != Item::LAVA_BUCKET.id
        && item.id != Item::POWDER_SNOW_BUCKET.id
        && world.dimension == Dimension::THE_NETHER
}

pub(crate) fn play_bucket_evaporation(world: &Arc<World>, position: &Vector3<f64>) {
    world.play_sound_raw(
        Sound::BlockFireExtinguish as u16,
        SoundCategory::Blocks,
        position,
        0.5,
        (rand::random::<f32>() - rand::random::<f32>()).mul_add(0.8, 2.6),
    );
}

async fn try_place_powder_snow(
    world: &Arc<World>,
    pos: BlockPos,
    direction: BlockDirection,
) -> bool {
    let state = world.get_block_state(&pos);
    let target_pos = if state.replaceable() {
        pos
    } else {
        pos.offset(direction.to_offset())
    };
    let target_state = world.get_block_state(&target_pos);
    if !target_state.is_air() && !target_state.is_liquid() && !target_state.replaceable() {
        return false;
    }
    world
        .set_block_state(
            &target_pos,
            Block::POWDER_SNOW.default_state.id,
            BlockFlags::NOTIFY_NEIGHBORS,
        )
        .await;
    true
}

pub(crate) async fn try_place_filled_bucket(
    world: &Arc<World>,
    item: &Item,
    pos: BlockPos,
    direction: BlockDirection,
) -> bool {
    let (block, state) = world.get_block_and_state(&pos);
    if item.id == Item::SULFUR_CUBE_BUCKET.id {
        // `MobBucketItem(EntityTypes.SULFUR_CUBE, Fluids.EMPTY, ...)` has no
        // block content to place.  `emptyContents` succeeds at the adjacent
        // position and `checkExtraContent` performs the actual spawn; trying
        // to treat it as a water bucket would create an unintended source.
        return true;
    }
    if item.id == Item::POWDER_SNOW_BUCKET.id {
        return try_place_powder_snow(world, pos, direction).await;
    }

    if is_waterlogged(block, state.id) && item.id == Item::WATER_BUCKET.id {
        let state_id = set_waterlogged(block, state.id, true);
        world
            .set_block_state(&pos, state_id, BlockFlags::NOTIFY_NEIGHBORS)
            .await;
        world.schedule_fluid_tick(&Fluid::WATER, pos, 5, TickPriority::Normal);
        return true;
    }

    let target_pos = pos.offset(direction.to_offset());
    let (block, state) = world.get_block_and_state(&target_pos);

    if waterlogged_check(block, state.id).is_some() {
        if item.id == Item::LAVA_BUCKET.id {
            return false;
        }
        let state_id = set_waterlogged(block, state.id, true);
        world
            .set_block_state(&target_pos, state_id, BlockFlags::NOTIFY_NEIGHBORS)
            .await;
        world.schedule_fluid_tick(&Fluid::WATER, target_pos, 5, TickPriority::Normal);
        return true;
    }

    if state.id == Block::AIR.default_state.id || state.is_liquid() {
        world
            .set_block_state(
                &target_pos,
                if item.id == Item::LAVA_BUCKET.id {
                    Block::LAVA.default_state.id
                } else {
                    Block::WATER.default_state.id
                },
                BlockFlags::NOTIFY_NEIGHBORS,
            )
            .await;
        return true;
    }

    false
}

impl ItemBehaviour for EmptyBucketItem {
    fn normal_use<'a>(
        &'a self,
        _block: &'a Item,
        player: &'a Player,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let world = player.world();
            let (start_pos, end_pos) = get_start_and_end_pos(player);

            let checker = async |pos: &BlockPos, world_inner: &Arc<World>| {
                let state_id = world_inner.get_block_state_id(pos);

                let block = Block::from_state_id(state_id);

                if state_id == Block::AIR.default_state.id {
                    return false;
                }

                (block.id != Block::WATER.id && block.id != Block::LAVA.id)
                    || ((block.id == Block::WATER.id && state_id == Block::WATER.default_state.id)
                        || (block.id == Block::LAVA.id && state_id == Block::LAVA.default_state.id))
            };

            let Some((block_pos, direction)) = world.raycast(start_pos, end_pos, checker).await
            else {
                return;
            };

            let Some(item) = try_pickup_bucket_item(&world, block_pos, direction).await else {
                return;
            };

            if let Some(server) = world.server.upgrade()
                && let Some(player_arc) = world.get_player_by_uuid(player.gameprofile.id)
            {
                let mut event =
                    crate::plugin::api::events::player::player_bucket::PlayerBucketFillEvent::new(
                        player_arc,
                        block_pos,
                        item.registry_key.to_string(),
                    );
                server.plugin_manager.fire(&server, &mut event).await;
                if event.cancelled {
                    return;
                }
            }

            give_player_bucket_item(player, item).await;
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl FilledBucketItem {
    async fn use_bucket(&self, stack: &ItemStack, player: &Player, hand: pumpkin_util::Hand) {
        let item = stack.item;
        let world = player.world();
        let (start_pos, end_pos) = get_start_and_end_pos(player);
        let checker = async |pos: &BlockPos, world_inner: &Arc<World>| {
            let state_id = world_inner.get_block_state_id(pos);
            if Fluid::from_state_id(state_id).is_some() {
                return false;
            }
            state_id != Block::AIR.default_state.id
        };

        let Some((pos, direction)) = world.raycast(start_pos, end_pos, checker).await else {
            return;
        };

        if should_evaporate_in_nether(item, &world) {
            play_bucket_evaporation(&world, &player.position());
            return;
        }
        if !try_place_filled_bucket(&world, item, pos, direction).await {
            return;
        }

        spawn_bucket_entity(&world, stack, pos.offset(direction.to_offset())).await;

        if let Some(server) = world.server.upgrade()
            && let Some(player_arc) = world.get_player_by_uuid(player.gameprofile.id)
        {
            let mut event =
                crate::plugin::api::events::player::player_bucket::PlayerBucketEmptyEvent::new(
                    player_arc,
                    pos,
                    item.registry_key.to_string(),
                );
            server.plugin_manager.fire(&server, &mut event).await;
        }

        if player.gamemode.load() != GameMode::Creative {
            let item_stack = ItemStack::new(1, &Item::BUCKET);
            player.inventory.set_stack_in_hand(hand, item_stack).await;
        }
    }
}

impl ItemBehaviour for FilledBucketItem {
    fn normal_use<'a>(
        &'a self,
        item: &'a Item,
        player: &'a Player,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let item = Item::from_id(item.id).unwrap_or(&Item::AIR);
            let stack = ItemStack::new(1, item);
            self.use_bucket(&stack, player, pumpkin_util::Hand::Right)
                .await;
        })
    }

    fn normal_use_with_hand<'a>(
        &'a self,
        _item: &'a Item,
        player: &'a Player,
        hand: pumpkin_util::Hand,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let stack = player.inventory().get_stack_in_hand(hand).await;
            self.use_bucket(&stack, player, hand).await;
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ItemBehaviour for MilkBucketItem {
    fn normal_use<'a>(
        &'a self,
        _item: &'a Item,
        player: &'a Player,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let stack = player.inventory().held_item().await;
            player
                .living_entity
                .set_active_hand(pumpkin_util::Hand::Right, stack, 32)
                .await;
        })
    }

    fn on_stopped_using<'a>(
        &'a self,
        _stack: &'a ItemStack,
        player: &'a Player,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            player.living_entity.reset_effects_and_attributes().await;
            give_player_bucket_item(player, &Item::BUCKET).await;
        })
    }

    fn get_use_duration(&self) -> i32 {
        32
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::bucket_entity_type;
    use pumpkin_data::{entity::EntityType, item::Item};

    #[test]
    fn sulfur_cube_bucket_uses_empty_content_entity_path() {
        assert_eq!(
            bucket_entity_type(&Item::SULFUR_CUBE_BUCKET).map(|entity| entity.id),
            Some(EntityType::SULFUR_CUBE.id)
        );
        assert_eq!(bucket_entity_type(&Item::WATER_BUCKET), None);
    }
}
