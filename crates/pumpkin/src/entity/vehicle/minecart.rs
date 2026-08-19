mod chest;
mod container;
mod furnace;
mod hopper;
mod rideable;
mod tnt;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

use pumpkin_protocol::java::server::play::SPlayerInput;
use rand::RngExt;

use crate::{
    block::entities::{
        BlockEntity, command_block::CommandBlockEntity, mob_spawner::MobSpawnerBlockEntity,
    },
    command::{CommandSender, context::command_source::CommandSource},
    entity::{
        Entity, EntityBase, EntityBaseFuture, NBTStorage, NbtFuture, living::LivingEntity,
        player::Player,
    },
    server::Server,
};
use pumpkin_data::Block;
use pumpkin_data::block_properties::{BlockProperties, PoweredRailLikeProperties};
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::meta_data_type::MetaDataType;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data::TrackedData;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_protocol::java::client::play::Metadata;
use pumpkin_util::GameMode;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::permission::PermissionLvl;
use pumpkin_util::text::TextComponent;
use pumpkin_world::inventory::Inventory;
use tokio::sync::Mutex;

use crate::entity::vehicle::vehicle::VehicleEntity;
use chest::ChestMinecart;
use container::MinecartInventory;
use furnace::FurnaceMinecart;
use hopper::HopperMinecart;
use rideable::RideableMinecart;
use tnt::TntMinecart;

/// State held by a command-block minecart.  The command is intentionally
/// independent of a block entity: the cart moves, while command execution
/// still needs a stable, serializable command source.
struct CommandMinecart {
    command: Mutex<String>,
    last_output: Mutex<String>,
    track_output: AtomicBool,
    success_count: AtomicU32,
    activation_cooldown: AtomicI32,
}

impl CommandMinecart {
    const ACTIVATION_DELAY: i32 = 4;

    fn new() -> Self {
        Self {
            command: Mutex::new(String::new()),
            last_output: Mutex::new(String::new()),
            track_output: AtomicBool::new(true),
            success_count: AtomicU32::new(0),
            activation_cooldown: AtomicI32::new(0),
        }
    }

    fn tick(&self) {
        let _ =
            self.activation_cooldown
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    (value > 0).then_some(value - 1)
                });
    }

    async fn activate(&self, entity: &Entity) {
        if self.activation_cooldown.load(Ordering::Relaxed) > 0
            || self
                .activation_cooldown
                .compare_exchange(
                    0,
                    Self::ACTIVATION_DELAY,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .is_err()
        {
            return;
        }

        let command = self.command.lock().await.trim().to_owned();
        if command.is_empty() {
            self.success_count.store(0, Ordering::Release);
            return;
        }

        let world = entity.world.load();
        let Some(server) = world.server.upgrade() else {
            self.success_count.store(0, Ordering::Release);
            return;
        };
        // Reuse the command-block sender semantics (permission level 2,
        // success count and LastOutput) while supplying the moving cart's
        // current position explicitly.  We do not register this temporary
        // object as a world block entity; it is only the command source state.
        let command_entity = Arc::new(CommandBlockEntity::new(
            entity.block_pos.load(),
            self.track_output.load(Ordering::Acquire),
            false,
        ));
        *command_entity.command.lock().await = command.clone();
        let source = CommandSource::new(
            CommandSender::CommandBlock(command_entity.clone(), world.clone()),
            world.clone(),
            None,
            entity.pos.load(),
            Vector2::new(entity.yaw.load(), entity.pitch.load()),
            "@".to_owned(),
            TextComponent::text("@"),
            server.clone(),
        );
        server
            .command_dispatcher
            .read()
            .await
            .handle_command(&source, &command)
            .await;
        self.success_count.store(
            command_entity.success_count.load(Ordering::Acquire),
            Ordering::Release,
        );
        let last_output = command_entity.last_output.lock().await.clone();
        *self.last_output.lock().await = last_output.clone();
        entity.send_meta_data(
            &[Metadata::new(
                TrackedData::ID_LAST_OUTPUT,
                MetaDataType::OPTIONAL_TEXT_COMPONENT,
                Some(TextComponent::text(last_output)),
            )],
            None,
        );
    }

    async fn write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_string("Command", self.command.lock().await.clone());
        nbt.put_string("LastOutput", self.last_output.lock().await.clone());
        nbt.put_bool("TrackOutput", self.track_output.load(Ordering::Acquire));
        nbt.put_int(
            "SuccessCount",
            self.success_count.load(Ordering::Acquire) as i32,
        );
    }

    async fn read_nbt(&self, nbt: &NbtCompound) {
        *self.command.lock().await = nbt.get_string("Command").unwrap_or("").to_owned();
        *self.last_output.lock().await = nbt.get_string("LastOutput").unwrap_or("").to_owned();
        self.track_output.store(
            nbt.get_bool("TrackOutput").unwrap_or(true),
            Ordering::Release,
        );
        self.success_count.store(
            nbt.get_int("SuccessCount").unwrap_or(0).max(0) as u32,
            Ordering::Release,
        );
    }
}

/// A moving mob spawner backed by the normal spawner state machine.  Keeping
/// the state in an `Arc` preserves weighted potentials and delay across cart
/// movement, while `set_position` supplies the current cart block each tick.
struct SpawnerMinecart {
    spawner: Mutex<MobSpawnerBlockEntity>,
}

impl SpawnerMinecart {
    fn new(position: BlockPos) -> Self {
        Self {
            spawner: Mutex::new(MobSpawnerBlockEntity::new(position, None)),
        }
    }

    async fn tick(&self, entity: &Entity) {
        let world = entity.world.load();
        let spawner = self.spawner.lock().await;
        spawner.set_position(entity.block_pos.load());
        spawner.tick(&world).await;
    }

    async fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.spawner.lock().await.write_entity_nbt(nbt);
    }

    async fn read_nbt(&self, nbt: &NbtCompound) {
        // Replace the complete typed state so custom SpawnData, weighted
        // potentials and all timing/configuration fields survive a load.
        let position = self.spawner.lock().await.get_position();
        *self.spawner.lock().await = MobSpawnerBlockEntity::from_nbt(nbt, position);
    }
}

const fn get_exits(
    shape: pumpkin_data::block_properties::RailShape,
) -> (Vector3<f64>, Vector3<f64>) {
    use pumpkin_data::block_properties::RailShape;
    match shape {
        RailShape::NorthSouth => (Vector3::new(0.0, 0.0, -1.0), Vector3::new(0.0, 0.0, 1.0)),
        RailShape::EastWest => (Vector3::new(-1.0, 0.0, 0.0), Vector3::new(1.0, 0.0, 0.0)),
        RailShape::AscendingEast => (Vector3::new(-1.0, 0.0, 0.0), Vector3::new(1.0, 1.0, 0.0)),
        RailShape::AscendingWest => (Vector3::new(1.0, 0.0, 0.0), Vector3::new(-1.0, 1.0, 0.0)),
        RailShape::AscendingNorth => (Vector3::new(0.0, 0.0, 1.0), Vector3::new(0.0, 1.0, -1.0)),
        RailShape::AscendingSouth => (Vector3::new(0.0, 0.0, -1.0), Vector3::new(0.0, 1.0, 1.0)),
        RailShape::SouthEast => (Vector3::new(0.0, 0.0, 1.0), Vector3::new(1.0, 0.0, 0.0)),
        RailShape::SouthWest => (Vector3::new(0.0, 0.0, 1.0), Vector3::new(-1.0, 0.0, 0.0)),
        RailShape::NorthWest => (Vector3::new(0.0, 0.0, -1.0), Vector3::new(-1.0, 0.0, 0.0)),
        RailShape::NorthEast => (Vector3::new(0.0, 0.0, -1.0), Vector3::new(1.0, 0.0, 0.0)),
    }
}

const GRAVITY: f64 = 0.04;
const RAIL_HEIGHT_OFFSET: f64 = 0.0625;

pub struct MinecartEntity {
    pub vehicle: VehicleEntity,
    kind: MinecartKind,
}

enum MinecartKind {
    Rideable(RideableMinecart),
    Chest(ChestMinecart),
    Furnace(FurnaceMinecart),
    Hopper(HopperMinecart),
    Tnt(TntMinecart),
    Command(CommandMinecart),
    Spawner(SpawnerMinecart),
    Other,
}

impl MinecartEntity {
    pub fn new(entity: Entity) -> Self {
        let kind = match entity.entity_type.id {
            id if id == EntityType::MINECART.id => MinecartKind::Rideable(RideableMinecart),
            id if id == EntityType::CHEST_MINECART.id => MinecartKind::Chest(ChestMinecart::new()),
            id if id == EntityType::FURNACE_MINECART.id => {
                MinecartKind::Furnace(FurnaceMinecart::new())
            }
            id if id == EntityType::HOPPER_MINECART.id => {
                MinecartKind::Hopper(HopperMinecart::new())
            }
            id if id == EntityType::TNT_MINECART.id => MinecartKind::Tnt(TntMinecart::new()),
            id if id == EntityType::COMMAND_BLOCK_MINECART.id => {
                MinecartKind::Command(CommandMinecart::new())
            }
            id if id == EntityType::SPAWNER_MINECART.id => {
                MinecartKind::Spawner(SpawnerMinecart::new(entity.block_pos.load()))
            }
            _ => MinecartKind::Other,
        };
        Self {
            vehicle: VehicleEntity::new(entity),
            kind,
        }
    }

    const fn container(&self) -> Option<&Arc<MinecartInventory>> {
        match &self.kind {
            MinecartKind::Chest(minecart) => Some(minecart.inventory()),
            MinecartKind::Hopper(minecart) => Some(minecart.inventory()),
            _ => None,
        }
    }

    /// Analog output exposed by a detector rail for storage minecarts.
    pub async fn detector_rail_comparator_output(&self) -> Option<u8> {
        if let MinecartKind::Command(minecart) = &self.kind {
            return Some(minecart.success_count.load(Ordering::Acquire).min(15) as u8);
        }
        let inventory = self.container()?;
        Some(crate::block::calculate_comparator_output(inventory.as_ref()).await)
    }

    /// Applies the Java command-minecart editor packet after the network
    /// layer has checked creative mode, permission and interaction distance.
    pub async fn set_command(&self, command: &str, track_output: bool) {
        let MinecartKind::Command(minecart) = &self.kind else {
            return;
        };
        let command = command.strip_prefix('/').unwrap_or(command);
        *minecart.command.lock().await = command.to_owned();
        minecart.track_output.store(track_output, Ordering::Release);
        self.vehicle.entity.send_meta_data(
            &[Metadata::new(
                TrackedData::ID_COMMAND_NAME,
                MetaDataType::STRING,
                command.to_owned(),
            )],
            None,
        );
    }

    const fn drop_item(&self) -> Option<&'static Item> {
        match &self.kind {
            MinecartKind::Chest(_) => Some(&Item::CHEST_MINECART),
            MinecartKind::Furnace(_) => Some(&Item::FURNACE_MINECART),
            MinecartKind::Hopper(_) => Some(&Item::HOPPER_MINECART),
            MinecartKind::Tnt(_) => Some(&Item::TNT_MINECART),
            MinecartKind::Rideable(_) | MinecartKind::Command(_) | MinecartKind::Spawner(_) => {
                Some(&Item::MINECART)
            }
            _ => None,
        }
    }
}

impl NBTStorage for MinecartEntity {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.vehicle.entity.write_nbt(nbt).await;
            match &self.kind {
                MinecartKind::Chest(minecart) => minecart.write_nbt(nbt).await,
                MinecartKind::Furnace(minecart) => minecart.write_nbt(nbt),
                MinecartKind::Hopper(minecart) => minecart.write_nbt(nbt).await,
                MinecartKind::Tnt(minecart) => minecart.write_nbt(nbt),
                MinecartKind::Command(minecart) => minecart.write_nbt(nbt).await,
                MinecartKind::Spawner(minecart) => minecart.write_nbt(nbt).await,
                MinecartKind::Rideable(_) | MinecartKind::Other => {}
            }
        })
    }

    fn read_nbt_non_mut<'a>(&'a self, nbt: &'a NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            self.vehicle.entity.read_nbt_non_mut(nbt).await;
            match &self.kind {
                MinecartKind::Chest(minecart) => minecart.read_nbt(nbt).await,
                MinecartKind::Furnace(minecart) => minecart.read_nbt(nbt),
                MinecartKind::Hopper(minecart) => minecart.read_nbt(nbt).await,
                MinecartKind::Tnt(minecart) => minecart.read_nbt(nbt),
                MinecartKind::Command(minecart) => minecart.read_nbt(nbt).await,
                MinecartKind::Spawner(minecart) => minecart.read_nbt(nbt).await,
                MinecartKind::Rideable(_) | MinecartKind::Other => {}
            }
        })
    }
}

impl EntityBase for MinecartEntity {
    #[allow(clippy::too_many_lines)]
    fn tick<'a>(
        &'a self,
        caller: &'a Arc<dyn EntityBase>,
        _server: &'a Server,
    ) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move {
            self.vehicle.tick();
            if let MinecartKind::Furnace(minecart) = &self.kind {
                minecart.tick(&self.vehicle.entity);
            }
            if let MinecartKind::Command(minecart) = &self.kind {
                minecart.tick();
            }
            if let MinecartKind::Spawner(minecart) = &self.kind {
                minecart.tick(&self.vehicle.entity).await;
            }

            let world = self.vehicle.entity.world.load();
            let pos = self.vehicle.entity.pos.load();
            let mut block_pos = BlockPos(Vector3::new(
                pos.x.floor() as i32,
                pos.y.floor() as i32,
                pos.z.floor() as i32,
            ));

            let mut block = world.get_block(&block_pos);
            let mut state_id = world.get_block_state_id(&block_pos);

            let mut is_powered_rail = block.id == Block::POWERED_RAIL.id;
            let mut is_activator_rail = block.id == Block::ACTIVATOR_RAIL.id;
            let mut is_on_rails = is_powered_rail
                || is_activator_rail
                || block.id == Block::RAIL.id
                || block.id == Block::DETECTOR_RAIL.id;

            // If not on rails at current Y level, check the block directly below
            if !is_on_rails {
                let below_block_pos = BlockPos(Vector3::new(
                    block_pos.0.x,
                    block_pos.0.y - 1,
                    block_pos.0.z,
                ));
                let below_block = world.get_block(&below_block_pos);
                if below_block.id == Block::RAIL.id
                    || below_block.id == Block::POWERED_RAIL.id
                    || below_block.id == Block::DETECTOR_RAIL.id
                    || below_block.id == Block::ACTIVATOR_RAIL.id
                {
                    block_pos = below_block_pos;
                    block = below_block;
                    state_id = world.get_block_state_id(&block_pos);
                    is_powered_rail = block.id == Block::POWERED_RAIL.id;
                    is_activator_rail = block.id == Block::ACTIVATOR_RAIL.id;
                    is_on_rails = true;
                }
            }

            if is_powered_rail || is_activator_rail {
                let props = PoweredRailLikeProperties::from_state_id(state_id, block);
                let powered = props.powered;

                if is_activator_rail && let MinecartKind::Hopper(minecart) = &self.kind {
                    minecart.set_enabled(!powered);
                }

                if powered {
                    if is_powered_rail {
                        let mut velocity = self.vehicle.entity.velocity.load();
                        let speed = velocity.length();
                        if speed > 0.01 {
                            let new_speed = (speed + 0.06).min(0.4);
                            velocity = velocity
                                .normalize()
                                .multiply(new_speed, new_speed, new_speed);
                            self.vehicle.entity.velocity.store(velocity);
                        } else {
                            let yaw = self.vehicle.entity.yaw.load();
                            let push_dir = Vector3::new(
                                -f64::from((yaw.to_radians()).sin()),
                                0.0,
                                f64::from((yaw.to_radians()).cos()),
                            );
                            self.vehicle
                                .entity
                                .velocity
                                .store(push_dir.multiply(0.1, 0.1, 0.1));
                        }
                        self.vehicle.entity.send_velocity();
                    } else if is_activator_rail {
                        match &self.kind {
                            MinecartKind::Tnt(minecart) => {
                                minecart.prime(&self.vehicle.entity, 80);
                            }
                            MinecartKind::Command(minecart) => {
                                minecart.activate(&self.vehicle.entity).await;
                            }
                            MinecartKind::Rideable(_) => {
                                let passengers =
                                    self.vehicle.entity.passengers.lock().await.clone();
                                for passenger in passengers {
                                    self.vehicle
                                        .entity
                                        .remove_passenger(passenger.get_entity().entity_id)
                                        .await;
                                }
                                if self.vehicle.get_hurt_time() == 0 {
                                    self.vehicle.set_hurt_dir(-self.vehicle.get_hurt_dir());
                                    self.vehicle.set_hurt_time(10);
                                    self.vehicle.set_damage(50.0);
                                    self.vehicle.send_wobble_metadata();
                                }
                            }
                            _ => {}
                        }
                    }
                } else if is_powered_rail {
                    let mut velocity = self.vehicle.entity.velocity.load();
                    velocity = velocity.multiply(0.5, 0.5, 0.5);
                    if velocity.length() < 0.01 {
                        velocity = Vector3::new(0.0, 0.0, 0.0);
                    }
                    self.vehicle.entity.velocity.store(velocity);
                    self.vehicle.entity.send_velocity();
                }
            }

            if let MinecartKind::Tnt(minecart) = &self.kind
                && minecart.tick(&self.vehicle.entity).await
            {
                return;
            }

            let mut velocity = self.vehicle.entity.velocity.load();

            let mut has_driver = false;
            let mut driver_input = 0;
            let mut driver_yaw = 0.0f32;

            {
                let passengers = self.vehicle.entity.passengers.lock().await;
                if let Some(passenger) = passengers.first()
                    && let Some(player) = passenger.get_player()
                {
                    driver_input = player.last_input.load(Ordering::Relaxed);
                    driver_yaw = player.get_entity().yaw.load();
                    has_driver = true;
                }
            }

            if has_driver && is_on_rails {
                let forward = driver_input & SPlayerInput::FORWARD != 0;
                let backward = driver_input & SPlayerInput::BACKWARD != 0;

                let mut force_dir = Vector3::new(0.0, 0.0, 0.0);
                if forward {
                    let yaw_rad = f64::from(driver_yaw).to_radians();
                    force_dir.x = -yaw_rad.sin();
                    force_dir.z = yaw_rad.cos();
                } else if backward {
                    let yaw_rad = f64::from(driver_yaw).to_radians();
                    force_dir.x = yaw_rad.sin();
                    force_dir.z = -yaw_rad.cos();
                }

                if forward || backward {
                    velocity.x += force_dir.x * 0.02;
                    velocity.z += force_dir.z * 0.02;

                    let speed = velocity.x.hypot(velocity.z);
                    if speed > 0.15 {
                        #[allow(clippy::suboptimal_flops)]
                        let old_speed = self
                            .vehicle
                            .entity
                            .velocity
                            .load()
                            .x
                            .hypot(self.vehicle.entity.velocity.load().z);

                        let max_speed = old_speed.clamp(0.15, 0.4);
                        if speed > max_speed {
                            velocity.x = (velocity.x / speed) * max_speed;
                            velocity.z = (velocity.z / speed) * max_speed;
                        }
                    }
                    self.vehicle.entity.velocity.store(velocity);
                    self.vehicle.entity.send_velocity();
                }
            }

            let mut velocity = self.vehicle.entity.velocity.load();

            if is_on_rails {
                use pumpkin_data::block_properties::RailLikeProperties;
                use pumpkin_data::block_properties::{RailShape, RailShapeStraight};

                let shape = if block.id == Block::RAIL.id {
                    let props = RailLikeProperties::from_state_id(state_id, block);
                    props.shape
                } else {
                    let props = PoweredRailLikeProperties::from_state_id(state_id, block);
                    match props.shape {
                        RailShapeStraight::NorthSouth => RailShape::NorthSouth,
                        RailShapeStraight::EastWest => RailShape::EastWest,
                        RailShapeStraight::AscendingEast => RailShape::AscendingEast,
                        RailShapeStraight::AscendingWest => RailShape::AscendingWest,
                        RailShapeStraight::AscendingNorth => RailShape::AscendingNorth,
                        RailShapeStraight::AscendingSouth => RailShape::AscendingSouth,
                    }
                };

                let pos = self.vehicle.entity.pos.load();
                let block_center_bottom = Vector3::new(
                    f64::from(block_pos.0.x) + 0.5,
                    f64::from(block_pos.0.y),
                    f64::from(block_pos.0.z) + 0.5,
                );

                let (exit0, exit1) = get_exits(shape);
                let exit0 = exit0.multiply(0.5, 0.5, 0.5);
                let exit1 = exit1.multiply(0.5, 0.5, 0.5);

                let in_corner = exit0.x != exit1.x && exit0.z != exit1.z;
                let mut target_position = pos;

                if in_corner {
                    let from0to1 = exit1 - exit0;
                    let from0topos = pos - block_center_bottom - exit0;
                    let dot_num = from0to1.dot(&from0topos);
                    let dot_den = from0to1.dot(&from0to1);
                    if dot_den != 0.0 {
                        let travel_vector_from0 = from0to1.multiply(
                            dot_num / dot_den,
                            dot_num / dot_den,
                            dot_num / dot_den,
                        );
                        target_position = block_center_bottom.add(&exit0).add(&travel_vector_from0);
                    }
                } else {
                    let z_snap = (exit0.x - exit1.x).abs() > 1e-5;
                    let x_snap = (exit0.z - exit1.z).abs() > 1e-5;
                    if x_snap {
                        target_position.x = block_center_bottom.x;
                    }
                    if z_snap {
                        target_position.z = block_center_bottom.z;
                    }
                }

                target_position.y = match shape {
                    RailShape::AscendingEast
                    | RailShape::AscendingWest
                    | RailShape::AscendingNorth
                    | RailShape::AscendingSouth => pos.y,
                    _ => f64::from(block_pos.0.y) + RAIL_HEIGHT_OFFSET,
                };
                // Keep all position-derived state in sync. Writing `pos` directly leaves
                // `block_pos`/`chunk_pos` stale, so an unloading chunk can save the cart at
                // its old location and the entity tracker can keep a client-side ghost.
                // `set_pos` also emits the normal cross-chunk tracking transition.
                self.vehicle.entity.set_pos(target_position);

                let horizontal_in_direction = Vector3::new(exit1.x, 0.0, exit1.z);
                let mut horizontal_out_direction = Vector3::new(exit0.x, 0.0, exit0.z);

                if velocity.dot(&horizontal_out_direction) < velocity.dot(&horizontal_in_direction)
                {
                    horizontal_out_direction = horizontal_in_direction;
                }

                let out_position = block_center_bottom.add(&horizontal_out_direction).add(
                    &horizontal_out_direction
                        .normalize()
                        .multiply(1e-5, 1e-5, 1e-5),
                );

                let mut towards_out = out_position - target_position;
                towards_out.y = 0.0;
                let towards_length = towards_out.length();
                if towards_length > 1e-5 {
                    towards_out = towards_out.normalize();
                    let speed = velocity.length();
                    velocity = towards_out.multiply(speed, speed, speed);
                }

                velocity.y = 0.0;
                self.vehicle.entity.velocity.store(velocity);
            } else if !self.vehicle.entity.on_ground.load(Ordering::Relaxed) {
                velocity.y -= GRAVITY;
                self.vehicle.entity.velocity.store(velocity);
            }

            if velocity.length() > 0.001 {
                self.move_entity(caller, velocity).await;

                if let MinecartKind::Tnt(minecart) = &self.kind
                    && self
                        .vehicle
                        .entity
                        .horizontal_collision
                        .load(Ordering::Relaxed)
                    && velocity.x.mul_add(velocity.x, velocity.z * velocity.z) >= 0.01
                {
                    minecart
                        .explode(
                            &self.vehicle.entity,
                            velocity.x.mul_add(velocity.x, velocity.z * velocity.z),
                        )
                        .await;
                    return;
                }

                let new_pos = self.vehicle.entity.pos.load();

                let passengers = self.vehicle.entity.passengers.lock().await;
                for passenger in passengers.iter() {
                    passenger.get_entity().set_pos(new_pos);
                }
                drop(passengers);

                self.vehicle.entity.send_pos_rot();

                #[allow(clippy::useless_let_if_seq)]
                let mut friction = 0.95; // Vanilla minecart air drag

                if is_on_rails {
                    let passengers = self.vehicle.entity.passengers.lock().await;
                    let has_passengers = !passengers.is_empty();
                    drop(passengers);
                    friction = if has_passengers { 0.99 } else { 0.96 };
                } else {
                    let below_block_pos = BlockPos(Vector3::new(
                        block_pos.0.x,
                        block_pos.0.y - 1,
                        block_pos.0.z,
                    ));
                    let below_block = world.get_block(&below_block_pos);

                    let is_on_ground = self.vehicle.entity.on_ground.load(Ordering::Relaxed)
                        || (below_block.id != Block::AIR.id
                            && below_block.id != Block::WATER.id
                            && below_block.id != Block::LAVA.id);
                    let is_in_water = self.vehicle.entity.touching_water.load(Ordering::Relaxed)
                        || below_block.id == Block::WATER.id;

                    if is_on_ground {
                        friction = 0.5;
                    } else if is_in_water {
                        friction = 0.95;
                    }
                }

                let mut next_vel =
                    if is_on_rails && let MinecartKind::Furnace(minecart) = &self.kind {
                        minecart.velocity(&self.vehicle.entity, velocity)
                    } else if is_on_rails && let Some(inventory) = self.container() {
                        container::velocity(&self.vehicle.entity, inventory, velocity).await
                    } else {
                        velocity.multiply(friction, friction, friction)
                    };
                if next_vel.length() < 0.005 {
                    next_vel = Vector3::new(0.0, 0.0, 0.0);
                }
                self.vehicle.entity.velocity.store(next_vel);
                if next_vel.length_squared() == 0.0 {
                    self.vehicle.entity.send_velocity();
                }
            }

            if let MinecartKind::Hopper(minecart) = &self.kind {
                minecart.tick(&self.vehicle.entity).await;
            }
        })
    }

    fn get_entity(&self) -> &Entity {
        &self.vehicle.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn get_entity_inventory(self: Arc<Self>) -> Option<Arc<dyn Inventory>> {
        self.container()
            .map(|inventory| inventory.clone() as Arc<dyn Inventory>)
    }

    fn is_pushable(&self) -> bool {
        true
    }

    #[allow(clippy::too_many_lines)]
    fn push<'a>(&'a self, entity: &'a Arc<dyn EntityBase>) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move {
            let self_entity = self.get_entity();
            let other_entity = entity.get_entity();

            if self_entity.no_clip.load(Ordering::Relaxed)
                || other_entity.no_clip.load(Ordering::Relaxed)
            {
                return;
            }

            {
                let passengers = self_entity.passengers.lock().await;
                if passengers
                    .iter()
                    .any(|p| p.get_entity().entity_id == other_entity.entity_id)
                {
                    return;
                }
            }
            {
                let passengers = other_entity.passengers.lock().await;
                if passengers
                    .iter()
                    .any(|p| p.get_entity().entity_id == self_entity.entity_id)
                {
                    return;
                }
            }

            let mut xa = other_entity.pos.load().x - self_entity.pos.load().x;
            let mut za = other_entity.pos.load().z - self_entity.pos.load().z;
            let mut dd = xa * xa + za * za;
            if dd >= 1.0E-4 {
                dd = dd.sqrt();
                xa /= dd;
                za /= dd;
                let mut pow = 1.0 / dd;
                if pow > 1.0 {
                    pow = 1.0;
                }
                xa *= pow;
                za *= pow;
                xa *= 0.1;
                za *= 0.1;
                xa *= 0.5;
                za *= 0.5;

                let is_other_minecart = other_entity.entity_type.id == EntityType::MINECART.id
                    || other_entity.entity_type.id == EntityType::CHEST_MINECART.id
                    || other_entity.entity_type.id == EntityType::COMMAND_BLOCK_MINECART.id
                    || other_entity.entity_type.id == EntityType::FURNACE_MINECART.id
                    || other_entity.entity_type.id == EntityType::HOPPER_MINECART.id
                    || other_entity.entity_type.id == EntityType::SPAWNER_MINECART.id
                    || other_entity.entity_type.id == EntityType::TNT_MINECART.id;

                if is_other_minecart {
                    let xo = self_entity.velocity.load().x;
                    let zo = self_entity.velocity.load().z;

                    let dir = Vector3::new(xo, 0.0, zo).normalize();
                    let facing = Vector3::new(
                        f64::from(self_entity.yaw.load().to_radians().cos()),
                        0.0,
                        f64::from(self_entity.yaw.load().to_radians().sin()),
                    )
                    .normalize();

                    let dot = dir.dot(&facing).abs();
                    if dot >= 0.8 {
                        let vel = self_entity.velocity.load();
                        let ovel = other_entity.velocity.load();

                        let is_self_furnace =
                            self_entity.entity_type.id == EntityType::FURNACE_MINECART.id;
                        let is_other_furnace =
                            other_entity.entity_type.id == EntityType::FURNACE_MINECART.id;

                        if is_other_furnace && !is_self_furnace {
                            self_entity.velocity.store(vel.multiply(0.2, 1.0, 0.2));
                            let mut new_self_vel = self_entity.velocity.load();
                            new_self_vel.x += ovel.x - xa;
                            new_self_vel.z += ovel.z - za;
                            self_entity.velocity.store(new_self_vel);
                            self_entity.send_velocity();

                            other_entity.velocity.store(ovel.multiply(0.95, 1.0, 0.95));
                            other_entity.send_velocity();
                        } else if !is_other_furnace && is_self_furnace {
                            other_entity.velocity.store(ovel.multiply(0.2, 1.0, 0.2));
                            let mut new_other_vel = other_entity.velocity.load();
                            new_other_vel.x += vel.x + xa;
                            new_other_vel.z += vel.z + za;
                            other_entity.velocity.store(new_other_vel);
                            other_entity.send_velocity();

                            self_entity.velocity.store(vel.multiply(0.95, 1.0, 0.95));
                            self_entity.send_velocity();
                        } else {
                            #[allow(clippy::manual_midpoint)]
                            let xdd = (ovel.x + vel.x) / 2.0;
                            #[allow(clippy::manual_midpoint)]
                            let zdd = (ovel.z + vel.z) / 2.0;

                            self_entity.velocity.store(vel.multiply(0.2, 1.0, 0.2));
                            let mut new_self_vel = self_entity.velocity.load();
                            new_self_vel.x += xdd - xa;
                            new_self_vel.z += zdd - za;
                            self_entity.velocity.store(new_self_vel);
                            self_entity.send_velocity();

                            other_entity.velocity.store(ovel.multiply(0.2, 1.0, 0.2));
                            let mut new_other_vel = other_entity.velocity.load();
                            new_other_vel.x += xdd + xa;
                            new_other_vel.z += zdd + za;
                            other_entity.velocity.store(new_other_vel);
                            other_entity.send_velocity();
                        }
                    }
                } else {
                    if !self_entity.has_passengers().await && self.is_pushable() {
                        let mut vel = self_entity.velocity.load();
                        vel.x -= xa;
                        vel.z -= za;
                        self_entity.velocity.store(vel);
                        self_entity.send_velocity();
                    }

                    if !other_entity.has_passengers().await && entity.is_pushable() {
                        let mut vel = other_entity.velocity.load();
                        vel.x += xa / 4.0;
                        vel.z += za / 4.0;
                        other_entity.velocity.store(vel);
                        other_entity.send_velocity();
                    }
                }
            }
        })
    }

    fn is_collidable(&self, _entity: Option<Box<dyn EntityBase>>) -> bool {
        true
    }

    fn init_data_tracker(&self) -> EntityBaseFuture<'_, ()> {
        Box::pin(async move {
            self.vehicle.send_wobble_metadata();
            if let MinecartKind::Furnace(minecart) = &self.kind {
                minecart.init_data_tracker(&self.vehicle.entity);
            }
            match &self.kind {
                MinecartKind::Command(minecart) => {
                    let command = minecart.command.lock().await.clone();
                    let last_output = minecart.last_output.lock().await.clone();
                    self.vehicle.entity.send_meta_data(
                        &[Metadata::new(
                            TrackedData::ID_CUSTOM_DISPLAY_BLOCK,
                            MetaDataType::BLOCK_STATE,
                            VarInt(i32::from(Block::COMMAND_BLOCK.default_state.id.as_u16())),
                        )],
                        None,
                    );
                    self.vehicle.entity.send_meta_data(
                        &[Metadata::new(
                            TrackedData::ID_COMMAND_NAME,
                            MetaDataType::STRING,
                            command,
                        )],
                        None,
                    );
                    self.vehicle.entity.send_meta_data(
                        &[Metadata::new(
                            TrackedData::ID_LAST_OUTPUT,
                            MetaDataType::OPTIONAL_TEXT_COMPONENT,
                            Some(TextComponent::text(last_output)),
                        )],
                        None,
                    );
                }
                MinecartKind::Spawner(_) => {
                    self.vehicle.entity.send_meta_data(
                        &[Metadata::new(
                            TrackedData::ID_CUSTOM_DISPLAY_BLOCK,
                            MetaDataType::BLOCK_STATE,
                            VarInt(i32::from(Block::SPAWNER.default_state.id.as_u16())),
                        )],
                        None,
                    );
                }
                _ => {}
            }
        })
    }

    fn can_hit(&self) -> bool {
        self.vehicle.entity.is_alive()
    }

    fn damage_with_context<'a>(
        &'a self,
        _caller: &'a dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        source: Option<&'a dyn EntityBase>,
        cause: Option<&'a dyn EntityBase>,
    ) -> EntityBaseFuture<'a, bool> {
        Box::pin(async move {
            let creative = source
                .and_then(EntityBase::get_player)
                .is_some_and(|player| player.gamemode.load() == GameMode::Creative);

            if let MinecartKind::Tnt(minecart) = &self.kind
                && damage_type == DamageType::ARROW
                && self.vehicle.entity.fire_ticks.load(Ordering::Relaxed) > 0
            {
                let projectile_speed_squared = cause
                    .map(|entity| entity.get_entity().velocity.load().length_squared())
                    .unwrap_or_default();
                minecart
                    .explode(&self.vehicle.entity, projectile_speed_squared)
                    .await;
                if self.vehicle.entity.is_removed() {
                    return true;
                }
            }

            let will_break = self.vehicle.entity.is_alive()
                && (creative || self.vehicle.get_damage() + amount * 10.0 > 40.0);

            if let MinecartKind::Tnt(minecart) = &self.kind
                && will_break
                && !creative
            {
                let velocity = self.vehicle.entity.velocity.load();
                let speed_squared = velocity.x.mul_add(velocity.x, velocity.z * velocity.z);
                let ignites = damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FIRE)
                    || damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_EXPLOSION)
                    || self.vehicle.entity.fire_ticks.load(Ordering::Relaxed) > 0;
                if ignites || speed_squared >= 0.01 {
                    self.vehicle.apply_damage_wobble(amount);
                    let fuse = rand::rng().random_range(0..20) + rand::rng().random_range(0..20);
                    if self
                        .vehicle
                        .entity
                        .world
                        .load()
                        .level_info
                        .load()
                        .game_rules
                        .tnt_explodes
                    {
                        minecart.prime(&self.vehicle.entity, fuse);
                    } else {
                        minecart.set_fuse(fuse);
                    }
                    return true;
                }
            }

            let damaged = self.vehicle.damage_with_context(amount, source).await;

            if will_break && !creative && self.vehicle.entity.is_removed() {
                let world = self.vehicle.entity.world.load();
                if world.level_info.load().game_rules.entity_drops {
                    let position = self.vehicle.entity.block_pos.load();
                    if let Some(container) = self.container()
                        && container.claim_drops()
                    {
                        container.unpack_loot().await;
                        let inventory: Arc<dyn Inventory> = container.clone();
                        world.scatter_inventory(&position, &inventory).await;
                    }
                    if let Some(item) = self.drop_item() {
                        world.drop_stack(&position, ItemStack::new(1, item)).await;
                    }
                }
            }

            damaged
        })
    }

    fn interact<'a>(
        &'a self,
        player: &'a Arc<Player>,
        item_stack: &'a mut ItemStack,
    ) -> EntityBaseFuture<'a, bool> {
        Box::pin(async move {
            match &self.kind {
                MinecartKind::Chest(minecart) => {
                    minecart.interact(&self.vehicle.entity, player).await
                }
                MinecartKind::Furnace(minecart) => {
                    minecart.interact(&self.vehicle.entity, player, item_stack)
                }
                MinecartKind::Hopper(minecart) => {
                    minecart.interact(&self.vehicle.entity, player).await
                }
                MinecartKind::Rideable(minecart) => {
                    minecart.interact(&self.vehicle.entity, player).await
                }
                MinecartKind::Command(_)
                    if player.permission_lvl.load().ge(&PermissionLvl::Two) =>
                {
                    // The Java client opens the command-minecart editor here.
                    // Until that dedicated packet exists, consume the
                    // interaction for authorized users rather than mounting
                    // the cart or letting a normal player edit it.
                    true
                }
                MinecartKind::Command(_)
                | MinecartKind::Spawner(_)
                | MinecartKind::Tnt(_)
                | MinecartKind::Other => false,
            }
        })
    }

    fn on_player_collision<'a>(&'a self, player: &'a Arc<Player>) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move {
            if self
                .vehicle
                .entity
                .passengers
                .lock()
                .await
                .iter()
                .any(|passenger| passenger.get_entity().entity_id == player.entity_id())
            {
                return;
            }

            if player.is_spectator() {
                return;
            }

            let player_pos = player.get_entity().pos.load();
            let minecart_pos = self.vehicle.entity.pos.load();

            let mut diff_x = minecart_pos.x - player_pos.x;
            let mut diff_z = minecart_pos.z - player_pos.z;

            let dist_sq = diff_x * diff_x + diff_z * diff_z;
            if dist_sq > 0.0001 {
                let dist = dist_sq.sqrt();
                diff_x /= dist;
                diff_z /= dist;

                let push_force = 0.1;
                let mut vel = self.vehicle.entity.velocity.load();
                vel.x += diff_x * push_force;
                vel.z += diff_z * push_force;

                let horizontal_speed = vel.x.hypot(vel.z);
                if horizontal_speed > 0.4 {
                    vel.x = (vel.x / horizontal_speed) * 0.4;
                    vel.z = (vel.z / horizontal_speed) * 0.4;
                }

                self.vehicle.entity.velocity.store(vel);
                self.vehicle.entity.send_velocity();
            }
        })
    }

    fn move_entity<'a>(
        &'a self,
        caller: &'a Arc<dyn EntityBase>,
        motion: Vector3<f64>,
    ) -> EntityBaseFuture<'a, ()> {
        Box::pin(async move {
            let to_position = self.vehicle.entity.pos.load().add(&motion);
            self.vehicle.entity.move_entity(caller, motion).await;
            let should_continue = self.push_entities(caller).await;
            if should_continue {
                let current_pos = self.vehicle.entity.pos.load();
                let back_motion = to_position.sub(&current_pos);
                self.vehicle.entity.move_entity(caller, back_motion).await;
            }
        })
    }

    fn as_nbt_storage(&self) -> &dyn NBTStorage {
        self
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::CommandMinecart;
    use futures::executor::block_on;
    use pumpkin_nbt::compound::NbtCompound;

    #[test]
    fn command_minecart_activation_cooldown_is_four_ticks() {
        let minecart = CommandMinecart::new();
        minecart
            .activation_cooldown
            .store(4, std::sync::atomic::Ordering::Relaxed);
        minecart.tick();
        assert_eq!(
            minecart
                .activation_cooldown
                .load(std::sync::atomic::Ordering::Relaxed),
            3
        );
        minecart.tick();
        minecart.tick();
        minecart.tick();
        assert_eq!(
            minecart
                .activation_cooldown
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn command_minecart_nbt_preserves_command_state() {
        let minecart = CommandMinecart::new();
        block_on(async {
            *minecart.command.lock().await = "say hello".to_owned();
            *minecart.last_output.lock().await = "ok".to_owned();
            minecart
                .success_count
                .store(7, std::sync::atomic::Ordering::Release);
            let mut nbt = NbtCompound::new();
            minecart.write_nbt(&mut nbt).await;

            let restored = CommandMinecart::new();
            restored.read_nbt(&nbt).await;
            assert_eq!(restored.command.lock().await.as_str(), "say hello");
            assert_eq!(restored.last_output.lock().await.as_str(), "ok");
            assert_eq!(
                restored
                    .success_count
                    .load(std::sync::atomic::Ordering::Acquire),
                7
            );
        });
    }

    #[test]
    fn command_minecart_nbt_preserves_track_output_flag() {
        let minecart = CommandMinecart::new();
        minecart
            .track_output
            .store(false, std::sync::atomic::Ordering::Release);
        block_on(async {
            let mut nbt = NbtCompound::new();
            minecart.write_nbt(&mut nbt).await;
            assert_eq!(nbt.get_bool("TrackOutput"), Some(false));

            let restored = CommandMinecart::new();
            restored.read_nbt(&nbt).await;
            assert!(
                !restored
                    .track_output
                    .load(std::sync::atomic::Ordering::Acquire)
            );
        });
    }
}
