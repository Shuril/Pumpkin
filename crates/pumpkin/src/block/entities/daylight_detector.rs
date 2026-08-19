use std::pin::Pin;
use std::sync::Arc;

use pumpkin_data::block_properties::BlockProperties;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;

use crate::world::{BlockFlags, World};

use super::BlockEntity;

type DaylightDetectorProperties = pumpkin_data::block_properties::DaylightDetectorLikeProperties;

pub struct DaylightDetectorBlockEntity {
    pub position: BlockPos,
}

impl BlockEntity for DaylightDetectorBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(_nbt: &NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        Self { position }
    }

    fn write_nbt<'a>(
        &'a self,
        _nbt: &'a mut NbtCompound,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn tick<'a>(&'a self, world: &'a Arc<World>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {
            if world.get_world_age().await % 20 == 0 && world.dimension.has_skylight {
                Self::update_power(world, &self.position).await;
            }
        })
    }
}

impl DaylightDetectorBlockEntity {
    pub const ID: &'static str = "minecraft:daylight_detector";

    #[must_use]
    pub const fn new(position: BlockPos) -> Self {
        Self { position }
    }

    pub async fn update_power(world: &Arc<World>, block_pos: &BlockPos) {
        let (block, state) = world.get_block_and_state(block_pos);
        let mut props = DaylightDetectorProperties::from_state_id(state.id, block);

        let inverted = props.inverted;
        let time_of_day = world.get_time_of_day().await;
        let power = signal_strength(
            world.get_effective_sky_brightness(block_pos),
            time_of_day,
            inverted,
        );
        if power != props.power {
            props.power = power;
            let state = props.to_state_id(block);
            world
                .clone()
                .set_block_state(block_pos, state, BlockFlags::NOTIFY_ALL)
                .await;
        }
    }
}

/// Vanilla's `EnvironmentAttributes.SUN_ANGLE` track uses a symmetric cubic
/// bezier easing (`(0.362, 0.241, 0.638, 0.759)`) rather than a linear clock.
/// The duplicate keyframe at tick 6000 makes the angle wrap from 360 to 0 at
/// noon; the two segments below reproduce that wrap without a discontinuity at
/// midnight.
fn sun_angle_radians(time: i64) -> f32 {
    use std::f32::consts::PI;

    let tick = time.rem_euclid(24_000) as f32;
    let alpha = if tick < 6_000.0 {
        (tick + 18_000.0) / 24_000.0
    } else {
        (tick - 6_000.0) / 24_000.0
    };
    symmetric_cubic_bezier(alpha) * (PI * 2.0)
}

fn symmetric_cubic_bezier(x: f32) -> f32 {
    // This is the same bounded Newton-Raphson + bisection solver used by
    // net.minecraft.util.EasingType.CubicBezier in 26.2.
    const X1: f32 = 0.362;
    const Y1: f32 = 0.241;
    const X2: f32 = 1.0 - X1;
    const Y2: f32 = 1.0 - Y1;
    let curve = |v1: f32, v2: f32, t: f32| {
        let a = 3.0 * v1 - 3.0 * v2 + 1.0;
        let b = -6.0 * v1 + 3.0 * v2;
        let c = 3.0 * v1;
        ((a * t + b) * t + c) * t
    };
    let gradient = |v1: f32, v2: f32, t: f32| {
        let a = 3.0 * v1 - 3.0 * v2 + 1.0;
        let b = -6.0 * v1 + 3.0 * v2;
        let c = 3.0 * v1;
        (3.0 * a * t + 2.0 * b) * t + c
    };

    let mut t = x.clamp(0.0, 1.0);
    for _ in 0..4 {
        let error = curve(X1, X2, t) - x;
        if error.abs() < 1.0e-5 {
            return curve(Y1, Y2, t);
        }
        let slope = gradient(X1, X2, t);
        if slope < 1.0e-5 {
            break;
        }
        t -= (error / slope).clamp(-0.25, 0.25);
    }

    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..32 {
        let error = curve(X1, X2, t) - x;
        if error.abs() < 1.0e-5 {
            break;
        }
        if error < 0.0 {
            low = t;
        } else {
            high = t;
        }
        t = (low + high) * 0.5;
    }
    curve(Y1, Y2, t)
}

fn signal_strength(sky_brightness: u8, time: i64, inverted: bool) -> u8 {
    use std::f32::consts::PI;

    let mut target = i32::from(sky_brightness);
    if inverted {
        target = 15 - target;
    } else if target > 0 {
        let angle = sun_angle_radians(time);
        let offset = if angle < PI { 0.0 } else { PI * 2.0 };
        target = (target as f32 * (angle + (offset - angle) * 0.2).cos()).round() as i32;
    }
    target.clamp(0, 15) as u8
}

#[cfg(test)]
mod tests {
    use super::{signal_strength, sun_angle_radians};
    use std::f32::consts::PI;

    #[test]
    fn sun_angle_uses_vanilla_noon_wrap_and_easing() {
        assert!((sun_angle_radians(6_000)).abs() < 1.0e-4);
        assert!((sun_angle_radians(18_000) - PI).abs() < 1.0e-4);
        assert!((sun_angle_radians(24_000) - sun_angle_radians(0)).abs() < 1.0e-4);
        // A linear clock would be exactly 3π/2 at midnight.  Vanilla's eased
        // track is intentionally earlier/later than that value.
        assert!((sun_angle_radians(0) - 1.5 * PI).abs() > 0.01);
    }

    #[test]
    fn signal_strength_matches_vanilla_daylight_detector_curve() {
        assert_eq!(signal_strength(15, 6_000, false), 15);
        assert_eq!(signal_strength(15, 18_000, false), 0);
        assert_eq!(signal_strength(15, 6_000, true), 0);
        assert_eq!(signal_strength(0, 12_000, false), 0);
    }
}
