//! Server-side game-event and vibration dispatch.
//!
//! Vanilla exposes a single event bus to blocks, entities and sensors.  The
//! dispatcher intentionally carries the source position and a stable vanilla
//! frequency so calibrated sensors can filter without depending on packet or
//! entity IDs.

use pumpkin_util::math::position::BlockPos;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameEventKind {
    BlockChange,
    BlockDestroy,
    BlockPlace,
    BlockAttach,
    BlockDetach,
    BlockActivate,
    BlockDeactivate,
    EntityPlace,
    ContainerOpen,
    ContainerClose,
    EntityInteract,
    EntityMove,
    Flap,
    Step,
    Swim,
    ProjectileLand,
    ProjectileShoot,
    HitGround,
    Splash,
    Bounce,
    ItemInteractFinish,
    InstrumentPlay,
    ItemInteractStart,
    JukeboxPlay,
    JukeboxStopPlay,
    EntityAction,
    ElytraGlide,
    Unequip,
    EntityDismount,
    Equip,
    EntityMount,
    EntityDamage,
    Drink,
    Eat,
    BlockOpen,
    BlockClose,
    PrimeFuse,
    FluidPickup,
    FluidPlace,
    EntityDie,
    Explode,
    LightningStrike,
    Teleport,
    NoteBlockPlay,
    Shear,
    Shriek,
    Resonance(u8),
    /// Emitted by an activated sculk sensor.  Vanilla assigns this event no
    /// vibration frequency; it exists specifically so shriekers can listen
    /// for sensor tendrils clicking without recursively activating ordinary
    /// sensors.
    SculkSensorTendrilsClicking,
}

/// Returns whether an item-use action is allowed to create a vibration.
/// Vanilla treats a missing `minecraft:use_effects` component as enabled.
#[must_use]
pub const fn item_allows_vibrations(interact_vibrations: Option<bool>) -> bool {
    match interact_vibrations {
        Some(value) => value,
        None => true,
    }
}

impl GameEventKind {
    /// Mojang's vibration frequency table (1..=15).  Unknown extension events
    /// must choose a deterministic value instead of silently dropping a pulse.
    #[must_use]
    pub const fn frequency(self) -> i32 {
        match self {
            Self::BlockChange => 11,
            Self::BlockDestroy | Self::FluidPickup => 12,
            Self::BlockPlace => 13,
            Self::FluidPlace => 13,
            Self::BlockAttach
            | Self::BlockActivate
            | Self::BlockOpen
            | Self::PrimeFuse
            | Self::NoteBlockPlay => 10,
            Self::BlockDetach | Self::BlockDeactivate | Self::BlockClose => 9,
            Self::EntityPlace | Self::LightningStrike | Self::Teleport => 14,
            Self::ContainerOpen => 10,
            Self::ContainerClose => 9,
            Self::EntityInteract | Self::Shear | Self::EntityMount => 6,
            Self::EntityMove | Self::Flap | Self::Step | Self::Swim => 1,
            Self::ProjectileLand | Self::HitGround | Self::Splash | Self::Bounce => 2,
            Self::ItemInteractStart
            | Self::ItemInteractFinish
            | Self::ProjectileShoot
            | Self::InstrumentPlay => 3,
            Self::JukeboxPlay => 10,
            Self::JukeboxStopPlay => 9,
            Self::EntityAction | Self::ElytraGlide | Self::Unequip => 4,
            Self::EntityDismount | Self::Equip => 5,
            Self::EntityDamage => 7,
            Self::Drink | Self::Eat => 8,
            Self::EntityDie | Self::Explode | Self::Shriek => 15,
            Self::SculkSensorTendrilsClicking => 0,
            Self::Resonance(frequency) => {
                if frequency < 1 {
                    1
                } else if frequency > 15 {
                    15
                } else {
                    frequency as i32
                }
            }
        }
    }
}

/// Returns the redstone level emitted by an ordinary sculk sensor at a given
/// squared distance.  Keeping this pure makes fixed-distance parity tests
/// cheap and prevents float rounding from changing event output.
#[must_use]
pub fn sensor_power(distance_squared: i32) -> u8 {
    sensor_power_with_radius(distance_squared, 8)
}

/// Vanilla's `VibrationSystem.getRedstoneStrengthForDistance`.  The distance
/// is measured in blocks and the falloff is scaled by the listener radius;
/// importantly, vanilla floors the scaled value (it does not round the
/// Euclidean distance first).
#[must_use]
pub fn sensor_power_with_radius(distance_squared: i32, listener_radius: i32) -> u8 {
    debug_assert!(listener_radius > 0);
    let distance = f64::from(distance_squared.max(0)).sqrt();
    let scaled = (15.0 / f64::from(listener_radius.max(1))) * distance;
    (15 - scaled.floor() as i32).clamp(1, 15) as u8
}

#[must_use]
pub fn within_sensor_range(source: &BlockPos, target: &BlockPos) -> Option<i32> {
    within_sensor_range_with_radius(source, target, 8)
}

#[must_use]
pub fn within_sensor_range_with_radius(
    source: &BlockPos,
    target: &BlockPos,
    radius: i32,
) -> Option<i32> {
    let dx = source.0.x - target.0.x;
    let dy = source.0.y - target.0.y;
    let dz = source.0.z - target.0.z;
    let distance_squared = dx * dx + dy * dy + dz * dz;
    (distance_squared <= radius * radius).then_some(distance_squared)
}

/// Vanilla's `VibrationSelector` replacement rule. A listener keeps at most
/// one candidate for the current game tick: the nearest vibration wins, and
/// equal-distance candidates are resolved by the higher frequency. Once a
/// candidate has been selected for an earlier tick it must not be replaced by
/// a later event while it is travelling to the sensor.
#[must_use]
pub fn should_replace_vibration(
    current: Option<(i64, i32, i64)>,
    candidate_distance_squared: i64,
    candidate_frequency: i32,
    candidate_game_tick: i64,
) -> bool {
    let Some((current_distance_squared, current_frequency, current_game_tick)) = current else {
        return true;
    };
    current_game_tick == candidate_game_tick
        && (candidate_distance_squared < current_distance_squared
            || (candidate_distance_squared == current_distance_squared
                && candidate_frequency > current_frequency))
}

#[cfg(test)]
mod tests {
    use super::{
        GameEventKind, item_allows_vibrations, sensor_power, sensor_power_with_radius,
        should_replace_vibration, within_sensor_range,
    };
    use pumpkin_util::math::position::BlockPos;

    #[test]
    fn power_falls_off_with_distance_and_never_reaches_zero() {
        assert_eq!(sensor_power(0), 15);
        assert_eq!(sensor_power(1), 14);
        assert_eq!(sensor_power(64), 1);
        assert_eq!(sensor_power_with_radius(256, 16), 1);
        assert_eq!(GameEventKind::BlockPlace.frequency(), 13);
    }

    #[test]
    fn vanilla_frequency_table_and_resonance_clamp_are_stable() {
        let expected = [
            (GameEventKind::BlockChange, 11),
            (GameEventKind::BlockDestroy, 12),
            (GameEventKind::BlockPlace, 13),
            (GameEventKind::BlockAttach, 10),
            (GameEventKind::BlockDetach, 9),
            (GameEventKind::EntityPlace, 14),
            (GameEventKind::EntityInteract, 6),
            (GameEventKind::EntityMove, 1),
            (GameEventKind::Flap, 1),
            (GameEventKind::ProjectileLand, 2),
            (GameEventKind::ProjectileShoot, 3),
            (GameEventKind::ItemInteractStart, 3),
            (GameEventKind::JukeboxPlay, 10),
            (GameEventKind::JukeboxStopPlay, 9),
            (GameEventKind::EntityAction, 4),
            (GameEventKind::EntityDismount, 5),
            (GameEventKind::EntityDamage, 7),
            (GameEventKind::Drink, 8),
            (GameEventKind::EntityDie, 15),
            (GameEventKind::SculkSensorTendrilsClicking, 0),
        ];
        for (event, frequency) in expected {
            assert_eq!(event.frequency(), frequency);
        }
        assert_eq!(GameEventKind::Resonance(0).frequency(), 1);
        assert_eq!(GameEventKind::Resonance(7).frequency(), 7);
        assert_eq!(GameEventKind::Resonance(255).frequency(), 15);
    }

    #[test]
    fn range_is_bounded_by_eight_blocks() {
        let source = BlockPos::new(0, 0, 0);
        assert_eq!(
            within_sensor_range(&source, &BlockPos::new(8, 0, 0)),
            Some(64)
        );
        assert_eq!(within_sensor_range(&source, &BlockPos::new(9, 0, 0)), None);
    }

    #[test]
    fn selector_prefers_nearest_then_higher_frequency_on_same_tick() {
        assert!(should_replace_vibration(None, 9, 1, 100));
        assert!(should_replace_vibration(Some((16, 1, 100)), 9, 1, 100));
        assert!(should_replace_vibration(Some((9, 1, 100)), 9, 3, 100));
        assert!(!should_replace_vibration(Some((9, 3, 100)), 9, 1, 100));
        assert!(!should_replace_vibration(Some((9, 1, 99)), 1, 15, 100));
    }

    #[test]
    fn item_vibration_component_defaults_to_enabled() {
        assert!(item_allows_vibrations(None));
        assert!(item_allows_vibrations(Some(true)));
        assert!(!item_allows_vibrations(Some(false)));
    }
}
