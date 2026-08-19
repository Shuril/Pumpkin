use std::sync::Arc;
use tokio::sync::Mutex;

use crate::block::blocks::redstone::block_receives_redstone_power;
use crate::block::entities::crafter::CrafterBlockEntity;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, BlockFuture, GetComparatorOutputArgs, NormalUseArgs, OnNeighborUpdateArgs,
    OnPlaceArgs, OnScheduledTickArgs, PlacedArgs,
};
use pumpkin_data::block_properties::{
    BlockProperties, CrafterLikeProperties, HorizontalFacing, Orientation,
};
use pumpkin_data::sound::Sound;
use pumpkin_data::translation;
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{BlockDirection, BlockStateId};
use pumpkin_inventory::crafting::recipes::RecipeInputInventory;
use pumpkin_inventory::generic_container_screen_handler::create_crafter_3x3;
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_inventory::screen_handler::{
    BoxFuture, InventoryPlayer, ScreenHandlerFactory, SharedScreenHandler,
};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::text::TextComponent;
use pumpkin_world::inventory::Inventory;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;

struct CrafterScreenFactory(Arc<dyn Inventory>);

impl ScreenHandlerFactory for CrafterScreenFactory {
    fn create_screen_handler<'a>(
        &'a self,
        sync_id: u8,
        player_inventory: &'a Arc<PlayerInventory>,
        _player: &'a dyn InventoryPlayer,
    ) -> BoxFuture<'a, Option<SharedScreenHandler>> {
        Box::pin(async move {
            let handler = create_crafter_3x3(sync_id, player_inventory, self.0.clone()).await;
            let screen_handler_arc = Arc::new(Mutex::new(handler));

            Some(screen_handler_arc as SharedScreenHandler)
        })
    }

    fn get_display_name(&self) -> TextComponent {
        pumpkin_macros::translate_cross!(
            translation::java::CONTAINER_CRAFTER,
            translation::bedrock::CONTAINER_CRAFTER
        )
    }
}

#[pumpkin_block("minecraft:crafter")]
pub struct CrafterBlock;

impl BlockBehaviour for CrafterBlock {
    fn normal_use<'a>(&'a self, args: NormalUseArgs<'a>) -> BlockFuture<'a, BlockActionResult> {
        Box::pin(async move {
            if let Some(block_entity) = args.world.get_block_entity(args.position)
                && let Some(inventory) = block_entity.get_inventory()
            {
                args.player
                    .open_handled_screen(&CrafterScreenFactory(inventory), Some(*args.position))
                    .await;
            }
            BlockActionResult::Success
        })
    }

    fn on_place<'a>(&'a self, args: OnPlaceArgs<'a>) -> BlockFuture<'a, BlockStateId> {
        Box::pin(async move {
            let mut props = CrafterLikeProperties::default(args.block);
            let facing = args.direction;
            let horizontal = args.player.living_entity.entity.get_horizontal_facing();
            props.orientation = match facing {
                BlockDirection::Down => match horizontal {
                    HorizontalFacing::North => Orientation::DownNorth,
                    HorizontalFacing::South => Orientation::DownSouth,
                    HorizontalFacing::East => Orientation::DownEast,
                    HorizontalFacing::West => Orientation::DownWest,
                },
                BlockDirection::Up => match horizontal {
                    HorizontalFacing::North => Orientation::UpNorth,
                    HorizontalFacing::South => Orientation::UpSouth,
                    HorizontalFacing::East => Orientation::UpEast,
                    HorizontalFacing::West => Orientation::UpWest,
                },
                BlockDirection::North => Orientation::NorthUp,
                BlockDirection::South => Orientation::SouthUp,
                BlockDirection::East => Orientation::EastUp,
                BlockDirection::West => Orientation::WestUp,
            };
            props.triggered = block_receives_redstone_power(args.world, args.position).await;
            props.to_state_id(args.block)
        })
    }

    fn placed<'a>(&'a self, args: PlacedArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let crafter_block_entity = CrafterBlockEntity::new(*args.position);
            args.world.add_block_entity(Arc::new(crafter_block_entity));
            let state = args.world.get_block_state(args.position);
            let props = CrafterLikeProperties::from_state_id(state.id, args.block);
            if props.triggered {
                args.world
                    .schedule_block_tick(args.block, *args.position, 4, TickPriority::Normal);
            }
        })
    }

    fn on_neighbor_update<'a>(&'a self, args: OnNeighborUpdateArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let powered = block_receives_redstone_power(args.world, args.position).await;
            let mut props = CrafterLikeProperties::from_state_id(
                args.world.get_block_state(args.position).id,
                args.block,
            );

            if powered && !props.triggered {
                props.triggered = true;
                args.world
                    .schedule_block_tick(args.block, *args.position, 4, TickPriority::Normal);
                args.world
                    .set_block_state(
                        args.position,
                        props.to_state_id(args.block),
                        BlockFlags::NOTIFY_LISTENERS,
                    )
                    .await;
                if let Some(entity) = args.world.get_block_entity(args.position)
                    && let Some(crafter) = entity.as_any().downcast_ref::<CrafterBlockEntity>()
                {
                    crafter.set_triggered(true);
                }
            } else if !powered && props.triggered {
                props.triggered = false;
                props.crafting = false;
                args.world
                    .set_block_state(
                        args.position,
                        props.to_state_id(args.block),
                        BlockFlags::NOTIFY_LISTENERS,
                    )
                    .await;
                if let Some(entity) = args.world.get_block_entity(args.position)
                    && let Some(crafter) = entity.as_any().downcast_ref::<CrafterBlockEntity>()
                {
                    crafter.set_triggered(false);
                }
            }
        })
    }

    fn on_scheduled_tick<'a>(&'a self, args: OnScheduledTickArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let mut props = CrafterLikeProperties::from_state_id(
                args.world.get_block_state(args.position).id,
                args.block,
            );

            let block_entity = args.world.get_block_entity(args.position);
            let result = if let (Some(entity), Some(server)) =
                (block_entity.as_ref(), args.world.server.upgrade())
            {
                if let Some(crafter) = entity.as_any().downcast_ref::<CrafterBlockEntity>() {
                    let result = crafter.craft_once(server.recipe_manager.as_ref()).await;
                    if result.is_some() {
                        crafter.set_crafting_ticks_remaining(6);
                    }
                    result
                } else {
                    None
                }
            } else {
                None
            };

            let output_position = Self::output_position(args.position, props.orientation);
            if let Some(stacks) = result {
                // Java enters the visible crafting state only after a recipe
                // was selected; failed redstone pulses never set CRAFTING.
                props.crafting = true;
                args.world
                    .set_block_state(
                        args.position,
                        props.to_state_id(args.block),
                        BlockFlags::NOTIFY_LISTENERS,
                    )
                    .await;
                for mut stack in stacks {
                    if let Some(entity) = args.world.get_block_entity(&output_position)
                        && let Some(inventory) = entity.get_inventory()
                    {
                        Self::insert_output(
                            inventory.as_ref(),
                            &mut stack,
                            Self::output_direction(props.orientation).opposite(),
                        )
                        .await;
                    }
                    // Vanilla falls back to an item entity when the adjacent
                    // container cannot accept a stack.  Do this independently
                    // for the recipe result and every remainder, preserving
                    // output order and preventing a remainder from being lost.
                    if !stack.is_empty() {
                        args.world.drop_stack(&output_position, stack).await;
                    }
                }
                args.world.play_sound(
                    Sound::BlockCrafterCraft,
                    pumpkin_data::sound::SoundCategory::Blocks,
                    &args.position.to_f64(),
                );
            } else {
                args.world.play_sound(
                    Sound::BlockCrafterFail,
                    pumpkin_data::sound::SoundCategory::Blocks,
                    &args.position.to_f64(),
                );
                args.world.sync_world_event(
                    WorldEvent::ParticlesShootSmoke,
                    *args.position,
                    Self::orientation_data(props.orientation),
                );
            }

            // Keep CRAFTING set for the six-tick CrafterBlockEntity animation.
            // The entity tick clears it at the exact deadline and refreshes
            // comparator neighbours; clearing it here would make the visible
            // state last zero ticks while the persisted timer kept running.
        })
    }

    fn get_comparator_output<'a>(
        &'a self,
        args: GetComparatorOutputArgs<'a>,
    ) -> BlockFuture<'a, Option<u8>> {
        Box::pin(async move {
            if let Some(block_entity) = args.world.get_block_entity(args.position) {
                let crafter = block_entity.as_any().downcast_ref::<CrafterBlockEntity>()?;

                let mut occupied = 0u8;
                for i in 0..9 {
                    let stack = crafter.get_stack(i).await;
                    if !stack.is_empty() || !crafter.is_slot_enabled(i) {
                        occupied += 1;
                    }
                }
                Some(occupied)
            } else {
                None
            }
        })
    }
}

impl CrafterBlock {
    async fn insert_output(
        inventory: &dyn Inventory,
        stack: &mut pumpkin_data::item_stack::ItemStack,
        direction: BlockDirection,
    ) {
        for slot in 0..inventory.size() {
            if stack.is_empty() {
                break;
            }
            let current = inventory.get_stack(slot).await;
            if !inventory.can_insert_from_hopper(slot, stack, direction) {
                continue;
            }
            if current.is_empty() {
                inventory.set_stack(slot, stack.clone()).await;
                *stack = pumpkin_data::item_stack::ItemStack::EMPTY.clone();
                break;
            }
            if !inventory.can_merge_from_hopper(slot, &current, stack, direction) {
                continue;
            }
            if current.are_items_and_components_equal(stack)
                && current.item_count < current.get_max_stack_size()
            {
                let room = current.get_max_stack_size() - current.item_count;
                let moved = room.min(stack.item_count);
                let mut updated = current;
                updated.increment(moved);
                stack.decrement(moved);
                inventory.set_stack(slot, updated).await;
            }
        }
    }

    const fn orientation_data(orientation: Orientation) -> i32 {
        match orientation {
            Orientation::DownEast
            | Orientation::DownNorth
            | Orientation::DownSouth
            | Orientation::DownWest => 0,
            Orientation::UpEast
            | Orientation::UpNorth
            | Orientation::UpSouth
            | Orientation::UpWest => 1,
            Orientation::NorthUp => 2,
            Orientation::SouthUp => 3,
            Orientation::WestUp => 4,
            Orientation::EastUp => 5,
        }
    }

    fn output_position(position: &BlockPos, orientation: Orientation) -> BlockPos {
        let offset = match orientation {
            Orientation::DownEast
            | Orientation::DownNorth
            | Orientation::DownSouth
            | Orientation::DownWest => Vector3::new(0, -1, 0),
            Orientation::UpEast
            | Orientation::UpNorth
            | Orientation::UpSouth
            | Orientation::UpWest => Vector3::new(0, 1, 0),
            Orientation::NorthUp => Vector3::new(0, 0, -1),
            Orientation::SouthUp => Vector3::new(0, 0, 1),
            Orientation::WestUp => Vector3::new(-1, 0, 0),
            Orientation::EastUp => Vector3::new(1, 0, 0),
        };
        position.offset(offset)
    }

    const fn output_direction(orientation: Orientation) -> BlockDirection {
        match orientation {
            Orientation::DownEast
            | Orientation::DownNorth
            | Orientation::DownSouth
            | Orientation::DownWest => BlockDirection::Down,
            Orientation::UpEast
            | Orientation::UpNorth
            | Orientation::UpSouth
            | Orientation::UpWest => BlockDirection::Up,
            Orientation::NorthUp => BlockDirection::North,
            Orientation::SouthUp => BlockDirection::South,
            Orientation::WestUp => BlockDirection::West,
            Orientation::EastUp => BlockDirection::East,
        }
    }
}
