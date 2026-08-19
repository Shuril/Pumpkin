use super::BlockEntity;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use std::pin::Pin;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct PendingVibration {
    pub power: u8,
    pub frequency: i32,
    pub delay: u8,
    pub source_entity: Option<Uuid>,
    pub distance_squared: i64,
    pub game_tick: i64,
}

pub struct SculkSensorBlockEntity {
    pub position: BlockPos,
    pub last_vibration_frequency: Mutex<i32>,
    pub pending_vibration: Mutex<Option<PendingVibration>>,
}

impl BlockEntity for SculkSensorBlockEntity {
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
        let last_vibration_frequency = nbt.get_int("last_vibration_frequency").unwrap_or(0);
        let pending_vibration = match (
            nbt.get_byte("vibration_power"),
            nbt.get_int("vibration_frequency"),
            nbt.get_byte("vibration_delay"),
        ) {
            (Some(power), Some(frequency), Some(delay)) if power > 0 => Some(PendingVibration {
                power: power as u8,
                frequency,
                delay: delay as u8,
                source_entity: nbt.get_uuid("vibration_source"),
                distance_squared: nbt.get_long("vibration_distance_squared").unwrap_or(0),
                game_tick: nbt.get_long("vibration_game_time").unwrap_or(0),
            }),
            _ => None,
        };
        Self {
            position,
            last_vibration_frequency: Mutex::new(last_vibration_frequency),
            pending_vibration: Mutex::new(pending_vibration),
        }
    }

    fn write_nbt<'a>(
        &'a self,
        nbt: &'a mut NbtCompound,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            nbt.put_int(
                "last_vibration_frequency",
                *self.last_vibration_frequency.lock().await,
            );
            if let Some(vibration) = *self.pending_vibration.lock().await {
                nbt.put_byte("vibration_power", vibration.power as i8);
                nbt.put_int("vibration_frequency", vibration.frequency);
                nbt.put_byte("vibration_delay", vibration.delay as i8);
                nbt.put_long("vibration_distance_squared", vibration.distance_squared);
                nbt.put_long("vibration_game_time", vibration.game_tick);
                if let Some(source) = vibration.source_entity {
                    nbt.put_uuid("vibration_source", source);
                }
            }
        })
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        nbt.put_int(
            "last_vibration_frequency",
            *self.last_vibration_frequency.try_lock().ok()?,
        );
        if let Some(vibration) = *self.pending_vibration.try_lock().ok()? {
            nbt.put_byte("vibration_power", vibration.power as i8);
            nbt.put_int("vibration_frequency", vibration.frequency);
            nbt.put_byte("vibration_delay", vibration.delay as i8);
            nbt.put_long("vibration_distance_squared", vibration.distance_squared);
            nbt.put_long("vibration_game_time", vibration.game_tick);
            if let Some(source) = vibration.source_entity {
                nbt.put_uuid("vibration_source", source);
            }
        }
        Some(nbt)
    }

    fn tick<'a>(
        &'a self,
        world: &'a std::sync::Arc<crate::world::World>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let due = {
                let mut pending = self.pending_vibration.lock().await;
                let Some(mut vibration) = *pending else {
                    return;
                };
                if vibration.delay > 0 {
                    vibration.delay -= 1;
                    *pending = Some(vibration);
                    return;
                }
                pending.take()
            };
            if let Some(vibration) = due {
                let block = world.get_block(&self.position);
                crate::block::blocks::redstone::sculk_sensor::SculkSensorBlock::trigger(
                    world,
                    &self.position,
                    block,
                    vibration.power,
                    vibration.frequency,
                    vibration.source_entity,
                )
                .await;
            }
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl SculkSensorBlockEntity {
    pub const ID: &'static str = "minecraft:sculk_sensor";
    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            last_vibration_frequency: Mutex::new(0),
            pending_vibration: Mutex::new(None),
        }
    }

    pub fn queue_vibration(
        &self,
        power: u8,
        frequency: i32,
        delay: u8,
        source_entity: Option<Uuid>,
        distance_squared: i64,
        game_tick: i64,
    ) -> bool {
        let Ok(mut pending) = self.pending_vibration.try_lock() else {
            // A listener that is currently being ticked still handled the
            // event. Returning true prevents the world dispatcher from
            // taking its legacy "missing block entity" immediate-trigger
            // fallback and bypassing travel time.
            return true;
        };
        let current = pending.as_ref().map(|vibration| {
            (
                vibration.distance_squared,
                vibration.frequency,
                vibration.game_tick,
            )
        });
        if !crate::world::game_event::should_replace_vibration(
            current,
            distance_squared,
            frequency,
            game_tick,
        ) {
            return true;
        }
        *pending = Some(PendingVibration {
            power,
            frequency,
            delay,
            source_entity,
            distance_squared,
            game_tick,
        });
        true
    }
}
