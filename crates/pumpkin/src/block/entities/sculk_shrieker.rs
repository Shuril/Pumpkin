use super::BlockEntity;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use std::pin::Pin;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct PendingShriek {
    pub delay: u8,
    pub source_entity: Uuid,
}

pub struct SculkShriekerBlockEntity {
    pub position: BlockPos,
    pub warning_level: Mutex<i32>,
    pub pending_shriek: Mutex<Option<PendingShriek>>,
}

impl BlockEntity for SculkShriekerBlockEntity {
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
        let warning_level = nbt.get_int("warning_level").unwrap_or(0);
        let pending_shriek = match (nbt.get_byte("event_delay"), nbt.get_uuid("event_source")) {
            (Some(delay), Some(source_entity)) => Some(PendingShriek {
                delay: delay as u8,
                source_entity,
            }),
            _ => None,
        };
        Self {
            position,
            warning_level: Mutex::new(warning_level),
            pending_shriek: Mutex::new(pending_shriek),
        }
    }

    fn write_nbt<'a>(
        &'a self,
        nbt: &'a mut NbtCompound,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            nbt.put_int("warning_level", *self.warning_level.lock().await);
            if let Some(pending) = *self.pending_shriek.lock().await {
                nbt.put_byte("event_delay", pending.delay as i8);
                nbt.put_uuid("event_source", pending.source_entity);
            }
        })
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        nbt.put_int("warning_level", *self.warning_level.try_lock().ok()?);
        if let Some(pending) = *self.pending_shriek.try_lock().ok()? {
            nbt.put_byte("event_delay", pending.delay as i8);
            nbt.put_uuid("event_source", pending.source_entity);
        }
        Some(nbt)
    }

    fn tick<'a>(
        &'a self,
        world: &'a std::sync::Arc<crate::world::World>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let due = {
                let mut pending = self.pending_shriek.lock().await;
                let Some(mut shriek) = *pending else {
                    return;
                };
                if shriek.delay > 0 {
                    shriek.delay -= 1;
                    *pending = Some(shriek);
                    return;
                }
                pending.take()
            };
            if let Some(shriek) = due {
                crate::block::blocks::sculk::sculk_shrieker::SculkShriekerBlock::try_activate_from(
                    world,
                    &self.position,
                    Some(shriek.source_entity),
                )
                .await;
            }
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl SculkShriekerBlockEntity {
    pub const ID: &'static str = "minecraft:sculk_shrieker";
    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            warning_level: Mutex::new(0),
            pending_shriek: Mutex::new(None),
        }
    }

    pub fn queue_shriek(&self, delay: u8, source_entity: Uuid) -> bool {
        let Ok(mut pending) = self.pending_shriek.try_lock() else {
            return true;
        };
        if pending.is_some() {
            // The listener is already processing a shriek. The dispatcher
            // must not activate it synchronously as a fallback.
            return true;
        }
        *pending = Some(PendingShriek {
            delay,
            source_entity,
        });
        true
    }
}
