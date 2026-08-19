use crate::block::entities::BlockEntity;
use crate::world::World;
use crossbeam::atomic::AtomicCell;
use pumpkin_data::block_properties::HorizontalFacing;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos};
use std::any::Any;
use std::pin::Pin;
use std::sync::Arc;

pub struct BellBlockEntity {
    pub position: BlockPos,
    pub last_side_hit: AtomicCell<Option<HorizontalFacing>>,
    pub ring_ticks: AtomicCell<i32>,
    pub ringing: AtomicCell<bool>,
    resonating: AtomicCell<bool>,
    resonate_time: AtomicCell<i32>,
}

impl BellBlockEntity {
    pub const ID: &'static str = "minecraft:bell";
    #[must_use]
    pub const fn new(position: BlockPos) -> Self {
        Self {
            position,
            last_side_hit: AtomicCell::new(None),
            ring_ticks: AtomicCell::new(0),
            resonate_time: AtomicCell::new(0),
            resonating: AtomicCell::new(false),
            ringing: AtomicCell::new(false),
        }
    }
    pub fn activate(&self, direction: HorizontalFacing) {
        self.last_side_hit.store(Some(direction));
        // A new block event starts a fresh resonance window.  Once a
        // resonance has completed, vanilla keeps the 40-tick sentinel until
        // the next ring so a single ring cannot retrigger every tick.
        self.resonate_time.store(0);
        self.resonating.store(false);
        if self.ringing.load() {
            self.ring_ticks.store(0);
        } else {
            self.ringing.store(true);
        }
    }
    /// Returns whether a living entity in the bell's 32 block hearing radius
    /// belongs to the vanilla `#minecraft:raiders` entity tag.
    ///
    /// The check deliberately uses the generated tag rather than a hand-made
    /// list: this keeps the behaviour in sync when the protocol/data version
    /// adds another raider type (and is exactly how vanilla resolves the tag).
    fn raiders_hear_bell(&self, world: &World) -> bool {
        let bell = self.position.to_f64();
        let hearing_box = BoundingBox::new(
            bell.add_raw(-32.0, -32.0, -32.0),
            bell.add_raw(33.0, 33.0, 33.0),
        );
        world
            .get_entities_at_box(&hearing_box)
            .into_iter()
            .any(|entity| {
                let base = entity.get_entity();
                base.is_alive()
                    && base
                        .entity_type
                        .has_tag(&tag::EntityType::MINECRAFT_RAIDERS)
                    && base.pos.load().squared_distance_to_vec(&bell) <= 32.0 * 32.0
            })
    }

    /// Apply the temporary glowing effect to every raider in the vanilla
    /// highlight radius.  Effects are applied asynchronously because the
    /// living-entity effect store is protected by a tokio mutex.
    async fn make_raiders_glow(&self, world: &World) {
        let bell = self.position.to_f64();
        let highlight_box = BoundingBox::new(
            bell.add_raw(-48.0, -48.0, -48.0),
            bell.add_raw(49.0, 49.0, 49.0),
        );
        let effect = pumpkin_data::potion::Effect {
            effect_type: &StatusEffect::GLOWING,
            duration: 60,
            amplifier: 0,
            ambient: false,
            show_particles: true,
            show_icon: true,
            blend: false,
        };
        for entity in world.get_entities_at_box(&highlight_box) {
            let base = entity.get_entity();
            if !base.is_alive()
                || !base
                    .entity_type
                    .has_tag(&tag::EntityType::MINECRAFT_RAIDERS)
                || base.pos.load().squared_distance_to_vec(&bell) > 48.0 * 48.0
            {
                continue;
            }
            if let Some(living) = entity.get_living_entity() {
                living.add_effect(effect.clone()).await;
            }
        }
    }
}

impl BlockEntity for BellBlockEntity {
    fn write_nbt<'a>(
        &'a self,
        _nbt: &'a mut NbtCompound,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {})
    }

    fn from_nbt(_nbt: &NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        Self::new(position)
    }

    fn tick<'a>(&'a self, world: &'a Arc<World>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            if self.ringing.load() {
                self.ring_ticks.fetch_add(1);
            }
            if self.ring_ticks.load() >= 50 {
                self.ringing.store(false);
                self.ring_ticks.store(0);
            }
            if self.ring_ticks.load() >= 5
                && self.resonate_time.load() == 0
                && self.raiders_hear_bell(world)
            {
                self.resonating.store(true);
                world.play_sound_fine(
                    Sound::BlockBellResonate,
                    SoundCategory::Blocks,
                    &self.position.to_f64(),
                    1.0,
                    1.0,
                );
            }

            if self.resonating.load() {
                if self.resonate_time.load() < 40 {
                    self.resonate_time.fetch_add(1);
                } else {
                    self.make_raiders_glow(world).await;
                    self.resonating.store(false);
                }
            }
        })
    }

    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::entity::EntityType;

    #[test]
    fn raider_tag_matches_vanilla_entity_tag() {
        for raider in [
            &EntityType::EVOKER,
            &EntityType::PILLAGER,
            &EntityType::RAVAGER,
            &EntityType::VINDICATOR,
            &EntityType::ILLUSIONER,
            &EntityType::WITCH,
        ] {
            assert!(raider.has_tag(&tag::EntityType::MINECRAFT_RAIDERS));
        }
        assert!(!EntityType::VILLAGER.has_tag(&tag::EntityType::MINECRAFT_RAIDERS));
        assert!(!EntityType::ZOMBIE.has_tag(&tag::EntityType::MINECRAFT_RAIDERS));
    }

    #[test]
    fn ring_activation_resets_resonance_window() {
        let bell = BellBlockEntity::new(BlockPos::new(0, 64, 0));
        bell.resonating.store(true);
        bell.resonate_time.store(40);
        bell.activate(HorizontalFacing::North);
        assert!(!bell.resonating.load());
        assert_eq!(bell.resonate_time.load(), 0);
        assert!(bell.ringing.load());
        assert_eq!(bell.ring_ticks.load(), 0);
    }
}
