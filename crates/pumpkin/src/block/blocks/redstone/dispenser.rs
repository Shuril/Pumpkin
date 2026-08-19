use rand::{Rng, RngExt, rng};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::block::blocks::redstone::block_receives_redstone_power;
use crate::block::blocks::respawn_anchor::RespawnAnchorBlock;
use crate::block::blocks::tnt::TNTBlock;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, BlockFuture, GetComparatorOutputArgs, NormalUseArgs, OnNeighborUpdateArgs,
    OnPlaceArgs, OnScheduledTickArgs, PlacedArgs,
};
use crate::entity::decoration::armor_stand::ArmorStandEntity;
use crate::entity::item::ItemEntity;
use crate::entity::projectile::ThrownItemEntity;
use crate::entity::projectile::arrow::{ArrowEntity, ArrowPickup};
use crate::entity::projectile::egg::EggEntity;
use crate::entity::projectile::experience_bottle::ExperienceBottleEntity;
use crate::entity::projectile::firework_rocket::FireworkRocketEntity;
use crate::entity::projectile::lingering_potion::LingeringPotionEntity;
use crate::entity::projectile::small_fireball::SmallFireballEntity;
use crate::entity::projectile::snowball::SnowballEntity;
use crate::entity::projectile::splash_potion::SplashPotionEntity;
use crate::entity::projectile::wind_charge::{WIND_CHARGE_GRAVITY, WindChargeEntity};
use crate::entity::tnt::TNTEntity;
use crate::entity::r#type::from_type;
use crate::entity::vehicle::boat::BoatEntity;
use crate::entity::vehicle::minecart::MinecartEntity;
use crate::entity::{Entity, EntityBase};
use crate::item::ItemMetadata;
use crate::item::items::boat::BoatItem;
use crate::item::items::brush::brush_suspicious_block;
use crate::item::items::bucket::{
    FilledBucketItem, bucket_entity_type, play_bucket_evaporation, should_evaporate_in_nether,
    spawn_bucket_entity, try_pickup_fluid_at, try_place_filled_bucket,
};
use crate::item::items::honeycomb::try_wax_block;
use crate::item::items::ignite::ignition::Ignition;
use crate::item::items::minecart::MinecartItem;
use crate::item::items::spawn_egg::apply_entity_variant;
use crate::world::World;

use crate::block::entities::dispenser::DispenserBlockEntity;
use pumpkin_data::block_properties::{
    BlockProperties, Facing, PoweredRailLikeProperties, RailLikeProperties,
    SkeletonSkullLikeProperties,
};
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::PotionContentsImpl;
use pumpkin_data::data_component_impl::{DataComponentImpl, EquippableImpl, IDSet};
use pumpkin_data::entity::{EntityType, entity_from_egg};
use pumpkin_data::fluid::Fluid;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag;
use pumpkin_data::tag::Taggable;
use pumpkin_data::translation;
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{Block, BlockDirection, BlockStateId, FacingExt};
use pumpkin_inventory::generic_container_screen_handler::create_generic_3x3;
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_inventory::screen_handler::{
    BoxFuture, InventoryPlayer, ScreenHandlerFactory, SharedScreenHandler,
};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::Difficulty;
use pumpkin_util::math::boundingbox::{BoundingBox, EntityDimensions};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::wrap_degrees;
use pumpkin_util::text::TextComponent;
use pumpkin_world::inventory::Inventory;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;

struct DispenserScreenFactory(Arc<dyn Inventory>);

impl ScreenHandlerFactory for DispenserScreenFactory {
    fn create_screen_handler<'a>(
        &'a self,
        sync_id: u8,
        player_inventory: &'a Arc<PlayerInventory>,
        _player: &'a dyn InventoryPlayer,
    ) -> BoxFuture<'a, Option<SharedScreenHandler>> {
        Box::pin(async move {
            let handler = create_generic_3x3(sync_id, player_inventory, self.0.clone()).await;
            let screen_handler_arc = Arc::new(Mutex::new(handler));

            Some(screen_handler_arc as SharedScreenHandler)
        })
    }

    fn get_display_name(&self) -> TextComponent {
        pumpkin_macros::translate_cross!(
            translation::java::CONTAINER_DISPENSER,
            translation::bedrock::CONTAINER_DISPENSER
        )
    }
}

#[pumpkin_block("minecraft:dispenser")]
pub struct DispenserBlock;

type DispenserLikeProperties = pumpkin_data::block_properties::DispenserLikeProperties;

struct DispenseContext<'a> {
    world: &'a Arc<World>,
    position: &'a BlockPos,
    facing: Facing,
}

impl<'a> DispenseContext<'a> {
    const fn new(args: &OnScheduledTickArgs<'a>, facing: Facing) -> Self {
        Self {
            world: args.world,
            position: args.position,
            facing,
        }
    }
}

fn triangle<R: Rng>(rng: &mut R, min: f64, max: f64) -> f64 {
    (rng.random::<f64>() - rng.random::<f64>()).mul_add(max, min)
}

const fn to_normal(facing: Facing) -> Vector3<f64> {
    match facing {
        Facing::North => Vector3::new(0., 0., -1.),
        Facing::East => Vector3::new(1., 0., 0.),
        Facing::South => Vector3::new(0., 0., 1.),
        Facing::West => Vector3::new(-1., 0., 0.),
        Facing::Up => Vector3::new(0., 1., 0.),
        Facing::Down => Vector3::new(0., -1., 0.),
    }
}

const fn to_data3d(facing: Facing) -> i32 {
    match facing {
        Facing::North => 2,
        Facing::East => 5,
        Facing::South => 3,
        Facing::West => 4,
        Facing::Up => 1,
        Facing::Down => 0,
    }
}

impl BlockBehaviour for DispenserBlock {
    fn normal_use<'a>(&'a self, args: NormalUseArgs<'a>) -> BlockFuture<'a, BlockActionResult> {
        Box::pin(async move {
            if let Some(block_entity) = args.world.get_block_entity(args.position)
                && let Some(inventory) = block_entity.get_inventory()
            {
                args.player
                    .open_handled_screen(&DispenserScreenFactory(inventory), Some(*args.position))
                    .await;
            }
            BlockActionResult::Success
        })
    }

    fn on_place<'a>(&'a self, args: OnPlaceArgs<'a>) -> BlockFuture<'a, BlockStateId> {
        Box::pin(async move {
            let mut props = DispenserLikeProperties::default(args.block);
            props.facing = args.player.get_entity().get_facing().opposite();
            props.triggered = block_receives_redstone_power(args.world, args.position).await
                || block_receives_redstone_power(args.world, &args.position.up()).await;
            props.to_state_id(args.block)
        })
    }

    fn placed<'a>(&'a self, args: PlacedArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let dispenser_block_entity = DispenserBlockEntity::new(*args.position);
            args.world
                .add_block_entity(Arc::new(dispenser_block_entity));
            let state = args.world.get_block_state(args.position);
            let props = DispenserLikeProperties::from_state_id(state.id, args.block);
            if props.triggered {
                args.world
                    .schedule_block_tick(args.block, *args.position, 4, TickPriority::Normal);
            }
        })
    }

    fn on_neighbor_update<'a>(&'a self, args: OnNeighborUpdateArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let powered = block_receives_redstone_power(args.world, args.position).await
                || block_receives_redstone_power(args.world, &args.position.up()).await;

            let mut props = DispenserLikeProperties::from_state_id(
                args.world.get_block_state(args.position).id,
                args.block,
            );

            if powered && !props.triggered {
                args.world
                    .schedule_block_tick(args.block, *args.position, 4, TickPriority::Normal);
                props.triggered = true;
                args.world
                    .set_block_state(
                        args.position,
                        props.to_state_id(args.block),
                        BlockFlags::NOTIFY_LISTENERS,
                    )
                    .await;
            } else if !powered && props.triggered {
                props.triggered = false;
                args.world
                    .set_block_state(
                        args.position,
                        props.to_state_id(args.block),
                        BlockFlags::NOTIFY_LISTENERS,
                    )
                    .await;
            }
        })
    }

    fn on_scheduled_tick<'a>(&'a self, args: OnScheduledTickArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            if let Some(block_entity) = args.world.get_block_entity(args.position) {
                let Some(dispenser) = block_entity.as_any().downcast_ref::<DispenserBlockEntity>()
                else {
                    return;
                };

                if let Some((slot_index, mut item)) = dispenser.get_random_slot().await {
                    let props = DispenserLikeProperties::from_state_id(
                        args.world.get_block_state(args.position).id,
                        args.block,
                    );
                    let ctx = DispenseContext::new(&args, props.facing);
                    Self::dispense(&ctx, dispenser, &mut item).await;
                    dispenser.set_stack(slot_index, item).await;
                    // Inventory contents are the comparator's analog input;
                    // no block-state change occurs when dispensing, so notify
                    // adjacent comparators explicitly like vanilla does.
                    args.world
                        .update_comparators(args.position, &Block::DISPENSER)
                        .await;
                } else {
                    args.world
                        .sync_world_event(WorldEvent::SoundDispenserFail, *args.position, 0);
                }
            }
        })
    }

    fn get_comparator_output<'a>(
        &'a self,
        args: GetComparatorOutputArgs<'a>,
    ) -> BlockFuture<'a, Option<u8>> {
        Box::pin(async move {
            if let Some(block_entity) = args.world.get_block_entity(args.position)
                && let Some(inventory) = block_entity.get_inventory()
            {
                Some(crate::block::calculate_comparator_output(inventory.as_ref()).await)
            } else {
                None
            }
        })
    }
}

impl DispenserBlock {
    // Velocity values match the vanilla dispenser projectile settings.
    const DEFAULT_PROJECTILE_POWER: f64 = 1.1;
    const DEFAULT_PROJECTILE_UNCERTAINTY: f64 = 6.0;
    const POTION_PROJECTILE_POWER: f64 = 1.375;
    const POTION_PROJECTILE_UNCERTAINTY: f64 = 3.0;
    // Fire charges and wind charges share these values.
    const FIREBALL_PROJECTILE_POWER: f64 = 1.0;
    const FIREBALL_PROJECTILE_UNCERTAINTY: f64 = 6.666_666_5;
    const FIREWORK_PROJECTILE_POWER: f64 = 0.5;
    const FIREWORK_PROJECTILE_UNCERTAINTY: f64 = 1.0;

    async fn dispense(
        ctx: &DispenseContext<'_>,
        dispenser: &DispenserBlockEntity,
        item: &mut ItemStack,
    ) {
        // The dispatch order mirrors vanilla's registered
        // `DispenseItemBehavior`s: equipment is attempted before projectile
        // and block-item fallbacks, so a wearable item is never accidentally
        // ejected as a generic drop.  Rare entity-bucket/component variants
        // are handled by the shared bucket path below.
        let arrows = [
            Item::ARROW.id,
            Item::TIPPED_ARROW.id,
            Item::SPECTRAL_ARROW.id,
        ];
        let boats = BoatItem::ids();

        // Carved pumpkins and wither skeleton skulls are registered before the
        // generic equipment behavior: their structure-specific placement must
        // run first, while a failed special placement falls back to equipping
        // the item on a living entity.
        let equipment_handled = if item.item.id == Item::CARVED_PUMPKIN.id {
            Self::dispense_carved_pumpkin(ctx, item).await
                || Self::dispense_equipment(ctx, item).await
        } else if item.item.id == Item::WITHER_SKELETON_SKULL.id {
            Self::dispense_wither_skull(ctx, item).await
                || Self::dispense_equipment(ctx, item).await
        } else {
            Self::dispense_equipment(ctx, item).await
        };
        if !equipment_handled {
            if arrows.contains(&item.item.id) {
                // Arrows
                Self::fire_arrow(ctx, item).await;
            } else if boats.contains(&item.item.id) {
                // Boats
                if !Self::dispense_boat(ctx, item).await {
                    Self::drop_item(ctx, item).await;
                }
            } else if MinecartItem::ids().contains(&item.item.id) {
                if !Self::dispense_minecart(ctx, item).await {
                    Self::drop_item(ctx, item).await;
                }
            } else if item.item.id == Item::ARMOR_STAND.id {
                // Armor stands
                if !Self::dispense_armor_stand(ctx, item).await {
                    Self::drop_item(ctx, item).await;
                }
            } else if item.item.id == Item::TNT.id {
                // TNT
                Self::dispense_tnt(ctx, item).await;
            } else if item.item.id == Item::SNOWBALL.id {
                Self::dispense_snowball(ctx, item).await;
            } else if item.item.id == Item::EGG.id {
                Self::dispense_egg(ctx, item).await;
            } else if item.item.id == Item::EXPERIENCE_BOTTLE.id {
                Self::dispense_experience_bottle(ctx, item).await;
            } else if item.item.id == Item::SPLASH_POTION.id {
                Self::dispense_splash_potion(ctx, item).await;
            } else if item.item.id == Item::LINGERING_POTION.id {
                Self::dispense_lingering_potion(ctx, item).await;
            } else if item.item.id == Item::POTION.id {
                if !Self::dispense_potion(ctx, dispenser, item).await {
                    Self::drop_item(ctx, item).await;
                }
            } else if item.item.id == Item::FIRE_CHARGE.id {
                Self::dispense_fire_charge(ctx, item).await;
            } else if item.item.id == Item::WIND_CHARGE.id {
                Self::dispense_wind_charge(ctx, item).await;
            } else if item.item.id == Item::FIREWORK_ROCKET.id {
                Self::dispense_firework_rocket(ctx, item).await;
            } else if item.item.id == Item::BUCKET.id {
                // Empty buckets pick up the fluid in front of the dispenser
                Self::dispense_empty_bucket(ctx, dispenser, item).await;
            } else if FilledBucketItem::ids().contains(&item.item.id) {
                // Filled buckets place their fluid in front of the dispenser
                Self::dispense_filled_bucket(ctx, item).await;
            } else if item.item.id == Item::FLINT_AND_STEEL.id {
                // Flint and steel light fires and prime TNT
                Self::dispense_flint_and_steel(ctx, item).await;
            } else if item.item.id == Item::HONEYCOMB.id {
                // Honeycombs wax copper blocks
                Self::dispense_honeycomb(ctx, item).await;
            } else if item.item.id == Item::GLOWSTONE.id {
                if !Self::dispense_glowstone(ctx, item).await {
                    Self::drop_item(ctx, item).await;
                }
            } else if item.item.id == Item::GLASS_BOTTLE.id {
                if !Self::dispense_glass_bottle(ctx, dispenser, item).await {
                    Self::drop_item(ctx, item).await;
                }
            } else if item.item.id == Item::BONE_MEAL.id {
                if !Self::dispense_bone_meal(ctx, item).await {
                    Self::drop_item(ctx, item).await;
                }
            } else if item.item.id == Item::SHEARS.id {
                // Shears first try living targets (sheep), then beehives.
                if !Self::dispense_shears(ctx, item).await {
                    Self::drop_item(ctx, item).await;
                }
            } else if item.item.id == Item::BRUSH.id {
                if !Self::dispense_brush(ctx, item).await {
                    if !brush_suspicious_block(ctx.world, Self::target_position(ctx)).await {
                        Self::drop_item(ctx, item).await;
                    } else {
                        let source_item = item.clone();
                        let _ = item.damage_item(1);
                        Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
                        Self::emit_item_game_event(
                            ctx,
                            Self::target_position(ctx),
                            crate::world::game_event::GameEventKind::ItemInteractFinish,
                            &source_item,
                        )
                        .await;
                    }
                }
            } else if Self::is_shulker_box_item(item.item.id) {
                // Shulker boxes use the special block-placement behavior rather
                // than being ejected as an ordinary BlockItem.
                if !Self::dispense_shulker_box(ctx, item).await {
                    Self::drop_item(ctx, item).await;
                }
            } else if entity_from_egg(item.item.id).is_some() {
                // Spawn eggs
                if !Self::dispense_spawn_egg(ctx, item).await {
                    Self::drop_item(ctx, item).await;
                }
            } else {
                // Default / Drop
                Self::drop_item(ctx, item).await;
            }
        }
    }

    /// Mirrors `EquipmentDispenseItemBehavior`: equip a wearable item on the
    /// first living entity in the one-block target box when the item is marked
    /// `dispensable` and the target satisfies its optional entity predicate.
    /// Returning false deliberately leaves the stack untouched so the caller's
    /// normal default behavior can eject it.
    async fn dispense_equipment(ctx: &DispenseContext<'_>, item: &mut ItemStack) -> bool {
        let Some(equippable) = item.get_data_component::<EquippableImpl>() else {
            return false;
        };
        if !equippable.dispensable {
            return false;
        }

        let target = Self::target_position(ctx).to_f64();
        let bounds = BoundingBox::new_from_pos(
            target.x,
            target.y,
            target.z,
            &EntityDimensions::new(1.0, 1.0, 1.0),
        );
        let Some(entity) = ctx
            .world
            .get_entities_at_box(&bounds)
            .into_iter()
            .find(|entity| {
                if entity.get_living_entity().is_none() {
                    return false;
                }
                match &equippable.allowed_entities {
                    None => true,
                    Some(IDSet::IDs(ids)) => {
                        ids.iter().any(|id| *id == entity.get_entity().entity_type)
                    }
                    Some(IDSet::Tag(tag)) => entity
                        .get_entity()
                        .entity_type
                        .is_tagged_with(tag)
                        .unwrap_or(false),
                }
            })
        else {
            return false;
        };

        let Some(living) = entity.get_living_entity() else {
            return false;
        };
        let slot = equippable.slot.clone();
        let mut equipment = living.entity_equipment.lock().await;
        if !equipment.get(&slot).is_empty() {
            return false;
        }
        let equipped = item.split(1);
        equipment.put(&slot, equipped.clone());
        drop(equipment);
        living.send_equipment_changes(&[(slot.clone(), equipped)]);
        if let Some(mob) = entity.get_mob() {
            let mut chances = living.equipment_drop_chances.lock().await;
            chances.insert(slot, 1.0);
            // A mob equipped by a dispenser is persistent in vanilla and must
            // not immediately despawn as a natural mob.
            mob.get_mob_entity()
                .living_entity
                .entity
                .age
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let source_item = item.clone();
        ctx.world
            .sync_world_event(WorldEvent::SoundDispenserDispense, *ctx.position, 0);
        Self::emit_item_game_event(
            ctx,
            Self::target_position(ctx),
            crate::world::game_event::GameEventKind::Equip,
            &source_item,
        )
        .await;
        true
    }

    fn projectile_spawn_position(ctx: &DispenseContext<'_>) -> Vector3<f64> {
        ctx.position
            .to_centered_f64()
            .add(&(to_normal(ctx.facing) * 0.7))
    }

    fn launch_thrown(
        ctx: &DispenseContext<'_>,
        thrown: &ThrownItemEntity,
        power: f64,
        uncertainty: f64,
    ) {
        let facing = to_normal(ctx.facing);
        thrown.set_velocity(facing.x, facing.y + 0.1, facing.z, power, uncertainty);
    }

    async fn finish_projectile_launch(
        ctx: &DispenseContext<'_>,
        projectile: Arc<dyn EntityBase>,
        launch_event: WorldEvent,
        source_item: &ItemStack,
    ) {
        // `PROJECTILE_SHOOT` is emitted at the authoritative launch boundary.
        // Passing the consumed stack through this common path is important:
        // `minecraft:use_effects.interact_vibrations=false` suppresses this
        // vibration for a custom dispenser item just as it does for a player
        // use, while the absent component keeps vanilla's enabled default.
        ctx.world
            .emit_game_event_from_item(
                *ctx.position,
                crate::world::game_event::GameEventKind::ProjectileShoot,
                None,
                source_item,
            )
            .await;
        ctx.world.spawn_entity(projectile).await;
        Self::play_dispense_effects(ctx, launch_event);
    }

    fn play_dispense_effects(ctx: &DispenseContext<'_>, sound_event: WorldEvent) {
        ctx.world.sync_world_event(sound_event, *ctx.position, 0);
        ctx.world.sync_world_event(
            WorldEvent::ParticlesShootSmoke,
            *ctx.position,
            to_data3d(ctx.facing),
        );
    }

    async fn emit_item_game_event(
        ctx: &DispenseContext<'_>,
        position: BlockPos,
        event: crate::world::game_event::GameEventKind,
        item: &ItemStack,
    ) {
        ctx.world
            .emit_game_event_from_item(position, event, None, item)
            .await;
    }

    async fn fire_arrow(ctx: &DispenseContext<'_>, item: &mut ItemStack) {
        let projectile = item.split(1);

        let facing = to_normal(ctx.facing);
        let arrow_entity = Entity::new(
            ctx.world.clone(),
            Self::projectile_spawn_position(ctx),
            ArrowEntity::entity_type_for_item(projectile.item),
        );
        let arrow =
            ArrowEntity::new_with_item(arrow_entity, None, &projectile, ArrowPickup::Allowed);

        arrow.set_velocity(
            facing.x,
            facing.y + 0.1,
            facing.z,
            Self::DEFAULT_PROJECTILE_POWER,
            Self::DEFAULT_PROJECTILE_UNCERTAINTY,
        );

        Self::finish_projectile_launch(
            ctx,
            Arc::new(arrow),
            WorldEvent::SoundDispenserProjectileLaunch,
            &projectile,
        )
        .await;
    }

    fn target_position(ctx: &DispenseContext<'_>) -> BlockPos {
        let facing = to_normal(ctx.facing);
        ctx.position.offset(Vector3::new(
            facing.x as i32,
            facing.y as i32,
            facing.z as i32,
        ))
    }

    fn has_room_for(
        ctx: &DispenseContext<'_>,
        spawn_pos: Vector3<f64>,
        size: &EntityDimensions,
    ) -> bool {
        let bounding_box = BoundingBox::new_from_pos(spawn_pos.x, spawn_pos.y, spawn_pos.z, size);
        ctx.world.is_space_empty(bounding_box)
            && ctx.world.get_entities_at_box(&bounding_box).is_empty()
    }

    async fn dispense_boat(ctx: &DispenseContext<'_>, item: &mut ItemStack) -> bool {
        let source_item = item.clone();
        let target = Self::target_position(ctx);
        let is_water = |id: u16| id == Fluid::WATER.id || id == Fluid::FLOWING_WATER.id;

        let spawn_pos = if is_water(ctx.world.get_fluid(&target).id) {
            target.to_f64()
        } else if ctx.world.get_block_state(&target).is_air()
            && is_water(ctx.world.get_fluid(&target.down()).id)
        {
            target.down().to_f64()
        } else {
            return false;
        };

        let entity_type = BoatItem::item_to_entity(item.item);
        let dimensions = EntityDimensions::new(
            entity_type.dimension[0],
            entity_type.dimension[1],
            entity_type.eye_height,
        );
        if !Self::has_room_for(ctx, spawn_pos, &dimensions) {
            return false;
        }

        let _ = item.split(1);
        let facing = to_normal(ctx.facing);
        let entity = Entity::new(ctx.world.clone(), spawn_pos, entity_type);
        entity.set_rotation(facing.x.atan2(facing.z) as f32 * 57.295_776, 0.0);
        ctx.world
            .spawn_entity(Arc::new(BoatEntity::new(entity)))
            .await;

        Self::emit_item_game_event(
            ctx,
            target,
            crate::world::game_event::GameEventKind::EntityPlace,
            &source_item,
        )
        .await;

        ctx.world
            .sync_world_event(WorldEvent::SoundDispenserDispense, *ctx.position, 0);
        true
    }

    /// Mirrors `MinecartDispenseItemBehavior`: a minecart is placed only on a
    /// rail in front of the dispenser or on the rail immediately below an air
    /// front block.  The exact 1.125 horizontal offset and slope Y offsets are
    /// observable with detector rails and prevent the old default-item eject.
    async fn dispense_minecart(ctx: &DispenseContext<'_>, item: &mut ItemStack) -> bool {
        let source_item = item.clone();
        let front = Self::target_position(ctx);
        let front_block = ctx.world.get_block(&front);
        let front_state = ctx.world.get_block_state_id(&front);
        let is_rail = front_block.has_tag(&tag::Block::MINECRAFT_RAILS);
        let rail_is_slope = |block: &Block, state_id: BlockStateId| {
            if PoweredRailLikeProperties::handles_block_id(block.id) {
                PoweredRailLikeProperties::from_state_id(state_id, block)
                    .shape
                    .is_ascending()
            } else if RailLikeProperties::handles_block_id(block.id) {
                RailLikeProperties::from_state_id(state_id, block)
                    .shape
                    .is_ascending()
            } else {
                false
            }
        };

        let y_offset = if is_rail {
            if rail_is_slope(front_block, front_state) {
                0.6
            } else {
                0.1
            }
        } else if front_block.is_air() {
            let below = front.down();
            let below_block = ctx.world.get_block(&below);
            if !below_block.has_tag(&tag::Block::MINECRAFT_RAILS) {
                return false;
            }
            if matches!(ctx.facing, Facing::Down)
                || !rail_is_slope(below_block, ctx.world.get_block_state_id(&below))
            {
                -0.9
            } else {
                -0.4
            }
        } else {
            return false;
        };

        let center = ctx.position.to_centered_f64();
        let normal = to_normal(ctx.facing);
        let spawn_pos = Vector3::new(
            center.x + normal.x * 1.125,
            ctx.position.0.y as f64 + normal.y + y_offset,
            center.z + normal.z * 1.125,
        );
        let entity_type = MinecartItem::item_to_entity(item.item);
        let entity = Entity::new(ctx.world.clone(), spawn_pos, entity_type);
        ctx.world
            .spawn_entity(Arc::new(MinecartEntity::new(entity)))
            .await;
        item.decrement(1);
        Self::emit_item_game_event(
            ctx,
            front,
            crate::world::game_event::GameEventKind::EntityPlace,
            &source_item,
        )
        .await;
        Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
        true
    }

    async fn dispense_armor_stand(ctx: &DispenseContext<'_>, item: &mut ItemStack) -> bool {
        let source_item = item.clone();
        let target = Self::target_position(ctx);
        let spawn_pos = target.to_f64();
        let dimensions = EntityDimensions::new(
            EntityType::ARMOR_STAND.dimension[0],
            EntityType::ARMOR_STAND.dimension[1],
            EntityType::ARMOR_STAND.eye_height,
        );
        if !Self::has_room_for(ctx, spawn_pos, &dimensions) {
            return false;
        }

        let _ = item.split(1);
        let facing = to_normal(ctx.facing);
        let entity = Entity::new(ctx.world.clone(), spawn_pos, &EntityType::ARMOR_STAND);
        entity.set_rotation(facing.x.atan2(facing.z) as f32 * 57.295_776, 0.0);

        ctx.world.play_sound(
            Sound::EntityArmorStandPlace,
            SoundCategory::Blocks,
            &spawn_pos,
        );
        ctx.world
            .spawn_entity(Arc::new(ArmorStandEntity::new(entity)))
            .await;

        Self::emit_item_game_event(
            ctx,
            target,
            crate::world::game_event::GameEventKind::EntityPlace,
            &source_item,
        )
        .await;

        ctx.world
            .sync_world_event(WorldEvent::SoundDispenserDispense, *ctx.position, 0);
        true
    }

    async fn dispense_tnt(ctx: &DispenseContext<'_>, item: &mut ItemStack) {
        const TNT_POWER: f32 = 4.0;
        const TNT_FUSE: u32 = 80;

        // DispenseItemBehavior is Optional: when `tnt_explodes` is disabled
        // it deliberately falls back to the ordinary item-eject behavior and
        // must not consume or prime the TNT.
        if !ctx.world.level_info.load().game_rules.tnt_explodes {
            Self::drop_item(ctx, item).await;
            return;
        }

        let source_item = item.clone();
        let _ = item.split(1);
        let spawn_pos = Self::target_position(ctx).to_f64();

        let entity = Entity::new(ctx.world.clone(), spawn_pos, &EntityType::TNT);
        let tnt = Arc::new(TNTEntity::new(entity, TNT_POWER, TNT_FUSE));
        ctx.world.spawn_entity(tnt).await;
        Self::emit_item_game_event(
            ctx,
            Self::target_position(ctx),
            crate::world::game_event::GameEventKind::EntityPlace,
            &source_item,
        )
        .await;
        ctx.world
            .play_sound(Sound::EntityTntPrimed, SoundCategory::Blocks, &spawn_pos);

        ctx.world
            .sync_world_event(WorldEvent::SoundDispenserDispense, *ctx.position, 0);
    }

    async fn dispense_spawn_egg(ctx: &DispenseContext<'_>, item: &mut ItemStack) -> bool {
        let source_item = item.clone();
        let Some(entity_type) = entity_from_egg(item.item.id) else {
            return false;
        };

        // SpawnEggDispenseItemBehavior delegates to SpawnUtil, which resolves
        // the centered target position and rejects a full collision volume.
        // Check before consuming so a wall/occupied target follows the normal
        // failed-dispense eject path instead of losing the egg.
        let target = Self::target_position(ctx);
        let spawn_pos = target.to_centered_f64();
        let dimensions = EntityDimensions::new(
            entity_type.dimension[0],
            entity_type.dimension[1],
            entity_type.eye_height,
        );
        if !Self::has_room_for(ctx, spawn_pos, &dimensions) {
            return false;
        }

        let _ = item.split(1);

        let mob = from_type(entity_type, spawn_pos, ctx.world, Uuid::new_v4());
        let yaw = wrap_degrees(rng().random::<f32>() * 360.0) % 360.0;
        mob.get_entity().set_rotation(yaw, 0.0);
        apply_entity_variant(item, mob.as_ref());

        ctx.world.spawn_entity(mob).await;

        Self::emit_item_game_event(
            ctx,
            target,
            crate::world::game_event::GameEventKind::EntityPlace,
            &source_item,
        )
        .await;

        ctx.world
            .sync_world_event(WorldEvent::SoundDispenserDispense, *ctx.position, 0);
        true
    }

    async fn dispense_snowball(ctx: &DispenseContext<'_>, item: &mut ItemStack) {
        let projectile = item.split(1);
        let entity = Entity::new(
            ctx.world.clone(),
            Self::projectile_spawn_position(ctx),
            &EntityType::SNOWBALL,
        );
        let snowball = SnowballEntity::new(entity);
        Self::launch_thrown(
            ctx,
            &snowball.thrown,
            Self::DEFAULT_PROJECTILE_POWER,
            Self::DEFAULT_PROJECTILE_UNCERTAINTY,
        );
        Self::finish_projectile_launch(
            ctx,
            Arc::new(snowball),
            WorldEvent::SoundDispenserProjectileLaunch,
            &projectile,
        )
        .await;
    }

    async fn dispense_egg(ctx: &DispenseContext<'_>, item: &mut ItemStack) {
        let projectile = item.split(1);
        let source_item = projectile.clone();
        let entity = Entity::new(
            ctx.world.clone(),
            Self::projectile_spawn_position(ctx),
            &EntityType::EGG,
        );
        let egg = EggEntity::new(entity);
        egg.set_item_stack(projectile).await;
        Self::launch_thrown(
            ctx,
            &egg.thrown,
            Self::DEFAULT_PROJECTILE_POWER,
            Self::DEFAULT_PROJECTILE_UNCERTAINTY,
        );
        Self::finish_projectile_launch(
            ctx,
            Arc::new(egg),
            WorldEvent::SoundDispenserProjectileLaunch,
            &source_item,
        )
        .await;
    }

    async fn dispense_experience_bottle(ctx: &DispenseContext<'_>, item: &mut ItemStack) {
        let projectile = item.split(1);
        let entity = Entity::new(
            ctx.world.clone(),
            Self::projectile_spawn_position(ctx),
            &EntityType::EXPERIENCE_BOTTLE,
        );
        let bottle = ExperienceBottleEntity::new(entity);
        Self::launch_thrown(
            ctx,
            &bottle.thrown,
            Self::POTION_PROJECTILE_POWER,
            Self::POTION_PROJECTILE_UNCERTAINTY,
        );
        Self::finish_projectile_launch(
            ctx,
            Arc::new(bottle),
            WorldEvent::SoundDispenserProjectileLaunch,
            &projectile,
        )
        .await;
    }

    async fn dispense_splash_potion(ctx: &DispenseContext<'_>, item: &mut ItemStack) {
        let projectile = item.split(1);
        let source_item = projectile.clone();
        let entity = Entity::new(
            ctx.world.clone(),
            Self::projectile_spawn_position(ctx),
            &EntityType::SPLASH_POTION,
        );
        let potion = SplashPotionEntity::new(entity);
        potion.set_item_stack(projectile).await;
        Self::launch_thrown(
            ctx,
            &potion.thrown,
            Self::POTION_PROJECTILE_POWER,
            Self::POTION_PROJECTILE_UNCERTAINTY,
        );
        Self::finish_projectile_launch(
            ctx,
            Arc::new(potion),
            WorldEvent::SoundDispenserProjectileLaunch,
            &source_item,
        )
        .await;
    }

    async fn dispense_lingering_potion(ctx: &DispenseContext<'_>, item: &mut ItemStack) {
        let projectile = item.split(1);
        let source_item = projectile.clone();
        let entity = Entity::new(
            ctx.world.clone(),
            Self::projectile_spawn_position(ctx),
            &EntityType::LINGERING_POTION,
        );
        let potion = LingeringPotionEntity::new(entity);
        potion.set_item_stack(projectile).await;
        Self::launch_thrown(
            ctx,
            &potion.thrown,
            Self::POTION_PROJECTILE_POWER,
            Self::POTION_PROJECTILE_UNCERTAINTY,
        );
        Self::finish_projectile_launch(
            ctx,
            Arc::new(potion),
            WorldEvent::SoundDispenserProjectileLaunch,
            &source_item,
        )
        .await;
    }

    /// Vanilla gives the ordinary potion item one special dispenser behavior:
    /// a water potion turns a convertable block (mud-capable dirt, rooted dirt,
    /// etc.) into mud and leaves a glass bottle in the dispenser.  Other potion
    /// variants use the normal item-eject behavior; they are deliberately not
    /// treated as splash potions because their component payload is not
    /// throwable in vanilla.
    async fn dispense_potion(
        ctx: &DispenseContext<'_>,
        dispenser: &DispenserBlockEntity,
        item: &mut ItemStack,
    ) -> bool {
        let Some(contents) = item.get_data_component::<PotionContentsImpl>() else {
            return false;
        };
        if contents.potion_id != Some(pumpkin_data::potion::Potion::WATER.id as i32) {
            return false;
        }

        let target = Self::target_position(ctx);
        let target_block = ctx.world.get_block(&target);
        if !target_block.has_tag(&tag::Block::MINECRAFT_CONVERTABLE_TO_MUD) {
            return false;
        }

        let source_item = item.clone();
        ctx.world
            .set_block_state(&target, Block::MUD.default_state.id, BlockFlags::NOTIFY_ALL)
            .await;
        item.decrement(1);
        let remainder = ItemStack::new(1, &Item::GLASS_BOTTLE);
        if let Some(remainder) = Self::add_to_first_free_slot(dispenser, remainder).await {
            Self::eject_item(ctx, remainder).await;
        }
        ctx.world.play_sound(
            Sound::ItemBottleEmpty,
            SoundCategory::Blocks,
            &ctx.position.to_f64(),
        );
        Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
        Self::emit_item_game_event(
            ctx,
            *ctx.position,
            crate::world::game_event::GameEventKind::FluidPlace,
            &source_item,
        )
        .await;
        true
    }

    /// The carved-pumpkin registration is an optional block behavior, not a
    /// generic BlockItem placement.  It may be dispensed only when the target
    /// is the top of a valid snow/iron golem base; otherwise vanilla falls back
    /// to the equipment behavior (already attempted before this branch).
    async fn dispense_carved_pumpkin(ctx: &DispenseContext<'_>, item: &mut ItemStack) -> bool {
        let target = Self::target_position(ctx);
        if !ctx.world.get_block_state(&target).is_air()
            || !Self::carved_pumpkin_has_golem_base(ctx.world, target)
        {
            return false;
        }

        let source_item = item.clone();
        ctx.world
            .set_block_state(
                &target,
                Block::CARVED_PUMPKIN.default_state.id,
                BlockFlags::NOTIFY_ALL,
            )
            .await;
        item.decrement(1);
        Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
        Self::emit_item_game_event(
            ctx,
            target,
            crate::world::game_event::GameEventKind::BlockPlace,
            &source_item,
        )
        .await;
        true
    }

    /// Mirrors the special wither-skull dispenser behavior. `set_block_state`
    /// invokes the registered `WitherSkeletonSkullBlock::placed` callback, so
    /// a valid three-skull soul-sand pattern runs the normal wither check.
    async fn dispense_wither_skull(ctx: &DispenseContext<'_>, item: &mut ItemStack) -> bool {
        let target = Self::target_position(ctx);
        // `WitherSkullBlock.canSpawnMob` is deliberately stricter than the
        // ordinary block-item path: the dispenser behavior is only successful
        // when the candidate is inside the build limits, the difficulty allows
        // a wither, and the lower soul-sand/soil base already exists.  An
        // empty target without that base must fall through to equipment
        // dispensing; placing a free-standing skull here would diverge from
        // vanilla and could consume the item unexpectedly.
        if !ctx.world.get_block_state(&target).is_air()
            || target.0.y < ctx.world.min_y.saturating_add(2)
            || ctx.world.level_info.load().difficulty == Difficulty::Peaceful
            || !Self::wither_skull_has_base(ctx.world, target)
        {
            return false;
        }

        let mut props = SkeletonSkullLikeProperties::default(&Block::WITHER_SKELETON_SKULL);
        props.rotation = match ctx.facing {
            Facing::South => 0,
            Facing::West => 4,
            Facing::North => 8,
            Facing::East => 12,
            Facing::Up | Facing::Down => 0,
        };
        let state_id = props.to_state_id(&Block::WITHER_SKELETON_SKULL);
        let source_item = item.clone();
        ctx.world
            .set_block_state(&target, state_id, BlockFlags::NOTIFY_ALL)
            .await;
        item.decrement(1);
        Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
        Self::emit_item_game_event(
            ctx,
            target,
            crate::world::game_event::GameEventKind::BlockPlace,
            &source_item,
        )
        .await;
        true
    }

    /// Checks the two horizontal orientations used by vanilla's
    /// `WitherSkullBlock.getOrCreateWitherBase` pattern.  The candidate skull
    /// may be the centre or either arm of the three-skull row, hence the three
    /// possible centre positions tested for each axis.  This is intentionally
    /// a base-only check; the placed callback performs the full seven-block
    /// pattern check after the skull is written.
    fn wither_skull_has_base(world: &Arc<World>, candidate: BlockPos) -> bool {
        let is_soul_base = |pos: BlockPos| {
            let block = world.get_block(&pos);
            block == &Block::SOUL_SAND || block == &Block::SOUL_SOIL
        };

        for dir in [BlockDirection::North, BlockDirection::West] {
            let opposite = dir.opposite();
            for offset in 0..3 {
                let center = match offset {
                    0 => candidate,
                    1 => candidate.offset(opposite.to_offset()),
                    2 => candidate.offset(dir.to_offset()),
                    _ => unreachable!(),
                };
                let top_middle = center.down();
                let base = top_middle.down();
                let arm1 = top_middle.offset(dir.to_offset());
                let arm2 = top_middle.offset(opposite.to_offset());

                if is_soul_base(top_middle)
                    && is_soul_base(base)
                    && is_soul_base(arm1)
                    && is_soul_base(arm2)
                {
                    return true;
                }
            }
        }
        false
    }

    fn carved_pumpkin_has_golem_base(world: &Arc<World>, top: BlockPos) -> bool {
        let below = top.down();
        let bottom = below.down();
        let snow = world.get_block(&below) == &Block::SNOW_BLOCK
            && world.get_block(&bottom) == &Block::SNOW_BLOCK;
        if snow {
            return true;
        }
        if world.get_block(&below) != &Block::IRON_BLOCK
            || world.get_block(&bottom) != &Block::IRON_BLOCK
        {
            return false;
        }

        // Iron golems accept either horizontal arm orientation.  The two
        // checks are equivalent to CarvedPumpkinBlock's rotated 3x3 pattern.
        [BlockDirection::North, BlockDirection::West]
            .into_iter()
            .any(|direction| {
                let opposite = direction.opposite();
                world.get_block(&below.offset(direction.to_offset())) == &Block::IRON_BLOCK
                    && world.get_block(&below.offset(opposite.to_offset())) == &Block::IRON_BLOCK
            })
    }

    async fn dispense_fire_charge(ctx: &DispenseContext<'_>, item: &mut ItemStack) {
        let projectile = item.split(1);
        let entity = Entity::new(
            ctx.world.clone(),
            Self::projectile_spawn_position(ctx),
            &EntityType::SMALL_FIREBALL,
        );
        let fireball = SmallFireballEntity::new(entity);
        // Vanilla aims fire charges straight along the facing axis, without the +0.1 Y bias
        // other projectiles get.
        let facing = to_normal(ctx.facing);
        fireball.thrown.set_velocity(
            facing.x,
            facing.y,
            facing.z,
            Self::FIREBALL_PROJECTILE_POWER,
            Self::FIREBALL_PROJECTILE_UNCERTAINTY,
        );
        Self::finish_projectile_launch(
            ctx,
            Arc::new(fireball),
            WorldEvent::SoundBlazeFireball,
            &projectile,
        )
        .await;
    }

    async fn dispense_wind_charge(ctx: &DispenseContext<'_>, item: &mut ItemStack) {
        let projectile = item.split(1);
        let entity = Entity::new(
            ctx.world.clone(),
            Self::projectile_spawn_position(ctx),
            &EntityType::WIND_CHARGE,
        );
        let thrown = ThrownItemEntity {
            entity,
            owner_id: None,
            collides_with_projectiles: false,
            has_hit: AtomicBool::new(false),
            gravity: WIND_CHARGE_GRAVITY,
        };
        Self::launch_thrown(
            ctx,
            &thrown,
            Self::FIREBALL_PROJECTILE_POWER,
            Self::FIREBALL_PROJECTILE_UNCERTAINTY,
        );
        Self::finish_projectile_launch(
            ctx,
            Arc::new(WindChargeEntity::new_normal(thrown)),
            WorldEvent::SoundWindChargeShoot,
            &projectile,
        )
        .await;
    }

    async fn dispense_firework_rocket(ctx: &DispenseContext<'_>, item: &mut ItemStack) {
        let projectile = item.split(1);
        let source_item = projectile.clone();
        let facing = to_normal(ctx.facing);
        // Vanilla spawns fireworks closer to the dispenser face and slightly above center.
        let position = ctx
            .position
            .to_centered_f64()
            .add(&(facing * (0.7 * 0.5125)))
            .add(&Vector3::new(0.0, 0.08, 0.0));
        let entity = Entity::new(ctx.world.clone(), position, &EntityType::FIREWORK_ROCKET);
        let rocket = FireworkRocketEntity::new_with_item(entity, projectile);

        // `FireworkRocketEntity` does not expose its inner projectile, so replicate
        // `ThrownItemEntity::set_velocity` here.
        let deviation = 0.017_227_5 * Self::FIREWORK_PROJECTILE_UNCERTAINTY;
        let velocity = Vector3::new(facing.x, facing.y + 0.1, facing.z)
            .normalize()
            .add_raw(
                triangle(&mut rng(), 0.0, deviation),
                triangle(&mut rng(), 0.0, deviation),
                triangle(&mut rng(), 0.0, deviation),
            )
            .multiply(
                Self::FIREWORK_PROJECTILE_POWER,
                Self::FIREWORK_PROJECTILE_POWER,
                Self::FIREWORK_PROJECTILE_POWER,
            );
        let rocket_entity = rocket.get_entity();
        rocket_entity.set_velocity(velocity);
        rocket_entity.set_rotation(
            velocity.x.atan2(velocity.z) as f32 * 57.295_776,
            velocity.y.atan2(velocity.horizontal_length()) as f32 * 57.295_776,
        );

        Self::finish_projectile_launch(
            ctx,
            Arc::new(rocket),
            WorldEvent::SoundFireworkShoot,
            &source_item,
        )
        .await;
    }

    async fn dispense_empty_bucket(
        ctx: &DispenseContext<'_>,
        dispenser: &DispenserBlockEntity,
        item: &mut ItemStack,
    ) {
        let front = Self::target_position(ctx);
        let source_item = item.clone();
        let Some(filled) = try_pickup_fluid_at(ctx.world, front).await else {
            Self::drop_item(ctx, item).await;
            return;
        };

        item.decrement(1);
        let filled_stack = ItemStack::new(1, filled);
        if item.is_empty() {
            *item = filled_stack;
        } else if let Some(rest) = Self::add_to_first_free_slot(dispenser, filled_stack).await {
            Self::eject_item(ctx, rest).await;
        }

        Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
        Self::emit_item_game_event(
            ctx,
            front,
            crate::world::game_event::GameEventKind::FluidPickup,
            &source_item,
        )
        .await;
    }

    /// Places `stack` into the first empty slot, returning it back if every slot is occupied.
    /// The slot currently being dispensed from is locked by the caller and skipped through
    /// `try_lock`; it cannot be empty anyway.
    async fn add_to_first_free_slot(
        dispenser: &DispenserBlockEntity,
        stack: ItemStack,
    ) -> Option<ItemStack> {
        let mut items = dispenser.items.write().await;
        for slot in items.iter_mut() {
            if slot.is_empty() {
                *slot = stack;
                dispenser.mark_dirty();
                return None;
            }
        }
        Some(stack)
    }

    async fn dispense_filled_bucket(ctx: &DispenseContext<'_>, item: &mut ItemStack) {
        let front = Self::target_position(ctx);
        let source_item = item.clone();

        let emptied = if should_evaporate_in_nether(item.item, ctx.world) {
            play_bucket_evaporation(ctx.world, &front.to_f64());
            true
        } else {
            try_place_filled_bucket(
                ctx.world,
                item.item,
                *ctx.position,
                ctx.facing.to_block_direction(),
            )
            .await
        };

        if emptied {
            if bucket_entity_type(item.item).is_some() {
                spawn_bucket_entity(ctx.world, item, front).await;
            }
            *item = ItemStack::new(1, &Item::BUCKET);
            Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
            Self::emit_item_game_event(
                ctx,
                front,
                crate::world::game_event::GameEventKind::FluidPlace,
                &source_item,
            )
            .await;
        } else {
            Self::drop_item(ctx, item).await;
        }
    }

    async fn dispense_flint_and_steel(ctx: &DispenseContext<'_>, item: &mut ItemStack) {
        let front = Self::target_position(ctx);
        let source_item = item.clone();
        let front_block = ctx.world.get_block(&front);

        let ignited = if front_block == &Block::TNT {
            TNTBlock::prime(ctx.world, &front).await;
            true
        } else {
            Ignition::ignite_block(
                |world: Arc<World>, pos: BlockPos, new_state_id: BlockStateId| async move {
                    world
                        .set_block_state(&pos, new_state_id, BlockFlags::NOTIFY_ALL)
                        .await;
                },
                ctx.world,
                front,
                front,
                front_block,
            )
            .await
        };

        if ignited {
            // `damage_item` already consumes the tool from the stack when it breaks.
            let _ = item.damage_item(1);
            Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
            Self::emit_item_game_event(
                ctx,
                front,
                crate::world::game_event::GameEventKind::PrimeFuse,
                &source_item,
            )
            .await;
        } else {
            Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserFail);
        }
    }

    async fn dispense_honeycomb(ctx: &DispenseContext<'_>, item: &mut ItemStack) {
        let front = Self::target_position(ctx);
        let source_item = item.clone();
        let front_block = ctx.world.get_block(&front);

        if try_wax_block(ctx.world, front, front_block).await {
            item.decrement(1);
            Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
            Self::emit_item_game_event(
                ctx,
                front,
                crate::world::game_event::GameEventKind::BlockChange,
                &source_item,
            )
            .await;
        } else {
            Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserFail);
        }
    }

    async fn dispense_glowstone(ctx: &DispenseContext<'_>, item: &mut ItemStack) -> bool {
        let target = Self::target_position(ctx);
        let source_item = item.clone();
        if ctx.world.get_block(&target).id != Block::RESPAWN_ANCHOR.id
            || !RespawnAnchorBlock::charge(ctx.world, &target, &Block::RESPAWN_ANCHOR).await
        {
            return false;
        }
        item.decrement(1);
        Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
        Self::emit_item_game_event(
            ctx,
            target,
            crate::world::game_event::GameEventKind::BlockChange,
            &source_item,
        )
        .await;
        true
    }

    /// Vanilla's `GlassBottleDispenseItemBehavior`: collect a water fluid or a
    /// full bee nest/hive and preserve the result in the dispenser inventory.
    /// A water cauldron is intentionally *not* a source for this behavior in
    /// Java 26.2 (its block fluid state is empty); a failed collection falls
    /// back to the ordinary item-eject behavior in the caller.
    async fn dispense_glass_bottle(
        ctx: &DispenseContext<'_>,
        dispenser: &DispenserBlockEntity,
        item: &mut ItemStack,
    ) -> bool {
        let pos = Self::target_position(ctx);
        let source_item = item.clone();
        let block = ctx.world.get_block(&pos);
        let state_id = ctx.world.get_block_state_id(&pos);
        let mut hive_facing = None;
        let (replacement, result_item, sound) = if block.id == Block::BEEHIVE.id
            || block.id == Block::BEE_NEST.id
        {
            let Some(props) = block.properties(state_id) else {
                return false;
            };
            let values = props.to_props();
            if !values
                .iter()
                .any(|(key, value)| *key == "honey_level" && *value == "5")
            {
                return false;
            }
            let new_values: Vec<(&str, &str)> = values
                .iter()
                .map(|(key, value)| {
                    if *key == "honey_level" {
                        (*key, "0")
                    } else {
                        (*key, *value)
                    }
                })
                .collect();
            hive_facing = Some(
                pumpkin_data::block_properties::BeeNestLikeProperties::from_state_id(
                    state_id, block,
                )
                .r#facing,
            );
            (
                Some(block.from_properties(&new_values).to_state_id(block)),
                &Item::HONEY_BOTTLE,
                Sound::ItemBottleFill,
            )
        } else if matches!(ctx.world.get_fluid(&pos).id, id if id == Fluid::WATER.id || id == Fluid::FLOWING_WATER.id)
        {
            // Vanilla's dispenser bottle behavior fills a water potion from
            // any water fluid state without removing the source block.  This
            // includes flowing and waterlogged states; water cauldrons do not
            // expose a water fluid state to this behavior in vanilla.
            (None, &Item::POTION, Sound::ItemBottleFill)
        } else {
            return false;
        };

        if let Some(replacement) = replacement {
            ctx.world
                .set_block_state(&pos, replacement, BlockFlags::NOTIFY_ALL)
                .await;
        }
        let hive_entity = ctx.world.get_block_entity(&pos);
        if let Some(facing) = hive_facing
            && let Some(hive) = hive_entity.as_ref().and_then(|entity| {
                entity
                    .as_any()
                    .downcast_ref::<crate::block::entities::beehive::BeehiveBlockEntity>()
            })
        {
            // `GlassBottleDispenseItemBehavior` releases bees before the
            // honey bottle is returned; otherwise a bottle silently traps
            // occupants forever after the first collection.
            hive.release_occupants(ctx.world, facing).await;
        }
        item.decrement(1);
        let result = if result_item.id == Item::POTION.id {
            ItemStack::new_with_component(
                1,
                result_item,
                vec![(
                    DataComponent::PotionContents,
                    Some(
                        pumpkin_data::data_component_impl::PotionContentsImpl {
                            potion_id: Some(pumpkin_data::potion::Potion::WATER.id as i32),
                            custom_color: None,
                            custom_effects: Vec::new(),
                            custom_name: None,
                        }
                        .to_dyn(),
                    ),
                )],
            )
        } else {
            ItemStack::new(1, result_item)
        };
        if let Some(rest) = Self::add_to_first_free_slot(dispenser, result).await {
            Self::eject_item(ctx, rest).await;
        }
        ctx.world
            .play_sound(sound, SoundCategory::Blocks, &pos.to_f64());
        Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
        Self::emit_item_game_event(
            ctx,
            pos,
            crate::world::game_event::GameEventKind::FluidPickup,
            &source_item,
        )
        .await;
        true
    }

    /// Mirrors the armadillo branch of vanilla's brush dispenser behavior.
    /// A successful action drops one scute, damages the brush by sixteen
    /// durability points, and leaves the stack in the dispenser. Returning
    /// `false` preserves the registered fallback to suspicious archaeology
    /// and finally the ordinary item eject.
    async fn dispense_brush(ctx: &DispenseContext<'_>, item: &mut ItemStack) -> bool {
        let target = Self::target_position(ctx).to_f64();
        let bounds = BoundingBox::new(target, target.add(&Vector3::new(1.0, 1.0, 1.0)));
        let Some(armadillo) = ctx
            .world
            .get_entities_at_box(&bounds)
            .into_iter()
            .find(|entity| entity.get_entity().entity_type.id == EntityType::ARMADILLO.id)
        else {
            return false;
        };

        let source_item = item.clone();
        let scute_position = armadillo.get_entity().pos.load();
        let scute = Arc::new(ItemEntity::new(
            Entity::new(ctx.world.clone(), scute_position, &EntityType::ITEM),
            ItemStack::new(1, &Item::ARMADILLO_SCUTE),
        ));
        ctx.world.spawn_entity(scute).await;
        let _ = item.damage_item(16);
        ctx.world.play_sound(
            Sound::EntityArmadilloBrush,
            SoundCategory::Neutral,
            &scute_position,
        );
        Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
        Self::emit_item_game_event(
            ctx,
            Self::target_position(ctx),
            crate::world::game_event::GameEventKind::ItemInteractFinish,
            &source_item,
        )
        .await;
        true
    }

    /// The dispenser variant of `BoneMealItem`.  It intentionally computes the
    /// complete new state before awaiting the world mutation, because block
    /// property trait objects are not `Send`.  Saplings are advanced to stage
    /// one here; tree feature growth is owned by the world-generation growth
    /// path and is not silently faked by a dispenser.
    async fn dispense_bone_meal(ctx: &DispenseContext<'_>, item: &mut ItemStack) -> bool {
        let pos = Self::target_position(ctx);
        let source_item = item.clone();
        let block = ctx.world.get_block(&pos);
        let state_id = ctx.world.get_block_state_id(&pos);
        let crop_state = (|| -> Option<BlockStateId> {
            let props = block.properties(state_id)?;
            let values = props.to_props();
            let (_, age) = values.iter().find(|(key, _)| *key == "age")?;
            let current_age = age.parse::<u8>().ok()?;
            let max_age = match block.id {
                id if id == Block::BEETROOTS.id || id == Block::SWEET_BERRY_BUSH.id => 3,
                id if id == Block::TORCHFLOWER_CROP.id => 1,
                id if id == Block::PITCHER_CROP.id => 4,
                id if id == Block::COCOA.id => 2,
                _ => 7,
            };
            (current_age < max_age).then(|| {
                let increment = if block.id == Block::COCOA.id {
                    1
                } else {
                    rng().random_range(2..=5)
                };
                let new_age = current_age
                    .saturating_add(increment)
                    .min(max_age)
                    .to_string();
                let new_values: Vec<(&str, &str)> = values
                    .iter()
                    .map(|(key, value)| {
                        if *key == "age" {
                            (*key, new_age.as_str())
                        } else {
                            (*key, *value)
                        }
                    })
                    .collect();
                block.from_properties(&new_values).to_state_id(block)
            })
        })();

        let sapling_state: Option<BlockStateId> = (|| {
            if crop_state.is_some() {
                return None;
            }
            let props = block.properties(state_id)?;
            let values = props.to_props();
            values
                .iter()
                .find(|(key, value)| *key == "stage" && *value == "0")?;
            let new_values: Vec<(&str, &str)> = values
                .iter()
                .map(|(key, value)| {
                    if *key == "stage" {
                        (*key, "1")
                    } else {
                        (*key, *value)
                    }
                })
                .collect();
            Some(block.from_properties(&new_values).to_state_id(block))
        })();

        let Some(new_state) = crop_state.or(sapling_state) else {
            return false;
        };
        ctx.world
            .set_block_state(&pos, new_state, BlockFlags::NOTIFY_ALL)
            .await;
        ctx.world
            .play_sound(Sound::ItemBoneMealUse, SoundCategory::Blocks, &pos.to_f64());
        item.decrement(1);
        Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
        Self::emit_item_game_event(
            ctx,
            pos,
            crate::world::game_event::GameEventKind::ItemInteractFinish,
            &source_item,
        )
        .await;
        true
    }

    async fn dispense_beehive_shears(
        ctx: &DispenseContext<'_>,
        item: &mut ItemStack,
        pos: BlockPos,
        block: &'static Block,
    ) -> bool {
        let source_item = item.clone();
        let state_id = ctx.world.get_block_state_id(&pos);
        let hive_entity = ctx.world.get_block_entity(&pos);
        let hive_facing =
            pumpkin_data::block_properties::BeeNestLikeProperties::from_state_id(state_id, block)
                .r#facing;
        // Materialize the state id before the first await; BlockProperties is
        // a trait object and is intentionally not carried across async points.
        let Some(new_state) = (|| {
            let props = block.properties(state_id)?;
            let values = props.to_props();
            if !values
                .iter()
                .any(|(key, value)| *key == "honey_level" && *value == "5")
            {
                return None;
            }
            let new_props: Vec<(&str, &str)> = values
                .iter()
                .map(|(key, value)| {
                    if *key == "honey_level" {
                        (*key, "0")
                    } else {
                        (*key, *value)
                    }
                })
                .collect();
            Some(block.from_properties(&new_props).to_state_id(block))
        })() else {
            return false;
        };
        ctx.world
            .set_block_state(&pos, new_state, BlockFlags::NOTIFY_ALL)
            .await;
        if let Some(hive) = hive_entity.as_ref().and_then(|entity| {
            entity
                .as_any()
                .downcast_ref::<crate::block::entities::beehive::BeehiveBlockEntity>()
        }) {
            // Bees leave through the nest's facing side, not necessarily the
            // direction the dispenser itself is pointing.
            hive.release_occupants(ctx.world, hive_facing).await;
        }
        ctx.world.play_sound(
            Sound::BlockBeehiveShear,
            SoundCategory::Blocks,
            &pos.to_f64(),
        );
        let drop = ItemEntity::new(
            Entity::new(ctx.world.clone(), pos.to_centered_f64(), &EntityType::ITEM),
            ItemStack::new(3, &Item::HONEYCOMB),
        );
        ctx.world.spawn_entity(Arc::new(drop)).await;
        let _ = item.damage_item(1);
        Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
        Self::emit_item_game_event(
            ctx,
            pos,
            crate::world::game_event::GameEventKind::Shear,
            &source_item,
        )
        .await;
        true
    }

    async fn dispense_shears(ctx: &DispenseContext<'_>, item: &mut ItemStack) -> bool {
        let source_item = item.clone();
        let pos = Self::target_position(ctx);
        let block = ctx.world.get_block(&pos);
        // Vanilla checks a full beehive before scanning entities in the target
        // box.  This matters when a hive has a collision-overlapping mob.
        if block.id == Block::BEEHIVE.id || block.id == Block::BEE_NEST.id {
            return Self::dispense_beehive_shears(ctx, item, pos, block).await;
        }
        let target = Self::target_position(ctx).to_f64();
        let bounds = BoundingBox::new_from_pos(
            target.x,
            target.y,
            target.z,
            &EntityDimensions::new(1.0, 1.0, 1.0),
        );
        if let Some(entity) = ctx
            .world
            .get_entities_at_box(&bounds)
            .into_iter()
            .find(|e| {
                e.get_mob()
                    .and_then(|mob| mob.get_sheep())
                    .is_some_and(|sheep| !sheep.is_sheared())
            })
        {
            let Some(sheep) = entity.get_mob().and_then(|mob| mob.get_sheep()) else {
                // The entity list can change between the eligibility scan and
                // this mutation (for example when a sheep despawns or a
                // plugin removes it).  Vanilla simply treats that dispense
                // attempt as a miss; never turn the race into a server panic.
                return false;
            };
            sheep.set_sheared(true);
            let pos = sheep.mob_entity.living_entity.entity.pos.load();
            ctx.world
                .play_sound(Sound::EntitySheepShear, SoundCategory::Neutral, &pos);
            let wool = match sheep.get_color() {
                0 => &Item::WHITE_WOOL,
                1 => &Item::ORANGE_WOOL,
                2 => &Item::MAGENTA_WOOL,
                3 => &Item::LIGHT_BLUE_WOOL,
                4 => &Item::YELLOW_WOOL,
                5 => &Item::LIME_WOOL,
                6 => &Item::PINK_WOOL,
                7 => &Item::GRAY_WOOL,
                8 => &Item::LIGHT_GRAY_WOOL,
                9 => &Item::CYAN_WOOL,
                10 => &Item::PURPLE_WOOL,
                11 => &Item::BLUE_WOOL,
                12 => &Item::BROWN_WOOL,
                13 => &Item::GREEN_WOOL,
                14 => &Item::RED_WOOL,
                _ => &Item::BLACK_WOOL,
            };
            let drop = ItemEntity::new(
                Entity::new(ctx.world.clone(), pos, &EntityType::ITEM),
                // Vanilla uses `nextInt(3) + 1`; use the range API instead of
                // reducing a byte modulo three (which gives a biased drop
                // distribution because 256 is not divisible by 3).
                ItemStack::new(rng().random_range(1..=3), wool),
            );
            ctx.world.spawn_entity(Arc::new(drop)).await;
            let _ = item.damage_item(1);
            Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
            Self::emit_item_game_event(
                ctx,
                Self::target_position(ctx),
                crate::world::game_event::GameEventKind::Shear,
                &source_item,
            )
            .await;
            return true;
        }

        false
    }

    #[inline]
    fn is_shulker_box_item(item_id: u16) -> bool {
        Block::from_item_id(item_id)
            .is_some_and(|block| block.is_tagged_with("minecraft:shulker_boxes") == Some(true))
    }

    /// Implements `ShulkerBoxDispenseBehavior` without manufacturing a fake
    /// player.  The block's normal placement callback is pure state
    /// construction for shulker boxes, so using the generated facing property
    /// here preserves the same orientation as `DirectionalPlaceContext`.
    async fn dispense_shulker_box(ctx: &DispenseContext<'_>, item: &mut ItemStack) -> bool {
        let source_item = item.clone();
        let Some(block) = Block::from_item_id(item.item.id) else {
            return false;
        };
        if !Self::is_shulker_box_item(item.item.id) {
            return false;
        }

        let position = Self::target_position(ctx);
        let current = ctx.world.get_block_state(&position);
        if !current.replaceable() {
            return false;
        }

        // Vanilla clicks the dispenser-facing side when the space below the
        // target is empty, otherwise the top side.  ShulkerBoxBlock's on-place
        // callback stores the opposite of that click direction in FACING.
        let below = BlockPos::new(position.0.x, position.0.y - 1, position.0.z);
        let click_face = if ctx.world.get_block(&below).is_air() {
            ctx.facing
        } else {
            Facing::Up
        };
        let mut properties = pumpkin_data::block_properties::EndRodLikeProperties::default(block);
        properties.facing = click_face;
        let state_id = properties.to_state_id(block);
        let state = pumpkin_data::BlockState::from_id(state_id);

        if !ctx.world.block_registry.can_place_at(
            None,
            Some(ctx.world),
            ctx.world.as_ref(),
            None,
            block,
            state,
            &position,
            Some(click_face.to_block_direction()),
            None,
        ) {
            return false;
        }

        ctx.world
            .set_block_state(&position, state_id, BlockFlags::NOTIFY_ALL)
            .await;

        // `set_block_state` invokes the normal placed callback, which creates
        // the shulker block entity.  Copy the modern container component into
        // that inventory so filled shulker boxes keep their contents.
        if let Some(container) = item
            .get_data_component::<pumpkin_data::data_component_impl::ContainerImpl>()
            .cloned()
            && let Some(block_entity) = ctx.world.get_block_entity(&position)
            && let Some(inventory) = block_entity.get_inventory()
        {
            for (slot, stack) in container.items {
                inventory.set_stack(slot as usize, stack).await;
            }
        }

        item.decrement(1);
        Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
        Self::emit_item_game_event(
            ctx,
            position,
            crate::world::game_event::GameEventKind::BlockPlace,
            &source_item,
        )
        .await;
        true
    }

    async fn drop_item(ctx: &DispenseContext<'_>, item: &mut ItemStack) {
        let drop_item = item.split(1);
        Self::eject_item(ctx, drop_item).await;
        Self::play_dispense_effects(ctx, WorldEvent::SoundDispenserDispense);
    }

    async fn eject_item(ctx: &DispenseContext<'_>, stack: ItemStack) {
        let facing = to_normal(ctx.facing);
        let mut position = ctx.position.to_centered_f64().add(&(facing * 0.7));

        position.y -= match ctx.facing {
            Facing::Up | Facing::Down => 0.125,
            _ => 0.15625,
        };

        let entity = Entity::new(ctx.world.clone(), position, &EntityType::ITEM);
        let rd = rng().random::<f64>().mul_add(0.1, 0.2);

        let velocity = Vector3::new(
            triangle(&mut rng(), facing.x * rd, 0.017_227_5 * 6.),
            triangle(&mut rng(), 0.2, 0.017_227_5 * 6.),
            triangle(&mut rng(), facing.z * rd, 0.017_227_5 * 6.),
        );

        let item_entity = Arc::new(ItemEntity::new_with_velocity(entity, stack, velocity, 40));
        ctx.world.spawn_entity(item_entity).await;
    }
}
