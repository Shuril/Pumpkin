use super::World;
use pumpkin_protocol::java::client::play::{CGameEvent, GameEvent};
use pumpkin_world::world_info::data_files::WeatherData;
use rand::RngExt;

// Weather timing constants
const RAIN_DELAY_MIN: i32 = 12_000;
const RAIN_DELAY_MAX: i32 = 180_000;
const RAIN_DURATION_MIN: i32 = 12_000;
const RAIN_DURATION_MAX: i32 = 24_000;
const THUNDER_DELAY_MIN: i32 = 12_000;
const THUNDER_DELAY_MAX: i32 = 180_000;
const THUNDER_DURATION_MIN: i32 = 3_600;
const THUNDER_DURATION_MAX: i32 = 15_600;

const WEATHER_TRANSITION_SPEED: f32 = 0.01;

pub struct Weather {
    pub clear_weather_time: i32,
    pub raining: bool,
    pub rain_time: i32,
    pub thundering: bool,
    pub thunder_time: i32,

    pub rain_level: f32,
    pub old_rain_level: f32,
    pub thunder_level: f32,
    pub old_thunder_level: f32,

    pub weather_cycle_enabled: bool,
}

impl Default for Weather {
    fn default() -> Self {
        Self::new()
    }
}

impl Weather {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            clear_weather_time: 0,
            raining: false,
            rain_time: 0,
            thundering: false,
            thunder_time: 0,
            rain_level: 0.0,
            old_rain_level: 0.0,
            thunder_level: 0.0,
            old_thunder_level: 0.0,
            weather_cycle_enabled: true,
        }
    }

    #[must_use]
    pub const fn from_data(data: &WeatherData) -> Self {
        Self {
            clear_weather_time: data.clear_weather_time,
            raining: data.raining,
            rain_time: data.rain_time,
            thundering: data.thundering,
            thunder_time: data.thunder_time,
            // Vanilla prepares the client-side levels from persisted flags.
            rain_level: if data.raining { 1.0 } else { 0.0 },
            old_rain_level: if data.raining { 1.0 } else { 0.0 },
            thunder_level: if data.thundering { 1.0 } else { 0.0 },
            old_thunder_level: if data.thundering { 1.0 } else { 0.0 },
            weather_cycle_enabled: true,
        }
    }

    #[must_use]
    pub const fn to_data(&self) -> WeatherData {
        WeatherData {
            rain_time: self.rain_time,
            raining: self.raining,
            thundering: self.thundering,
            thunder_time: self.thunder_time,
            clear_weather_time: self.clear_weather_time,
            data_version: 0,
        }
    }

    pub fn set_weather_parameters(
        &mut self,
        world: &World,
        clear_time: i32,
        rain_time: i32,
        raining: bool,
        thundering: bool,
    ) {
        let was_raining = self.raining;

        self.clear_weather_time = clear_time;
        self.rain_time = rain_time;
        self.thunder_time = rain_time;
        self.raining = raining;
        self.thundering = thundering;

        if was_raining != raining {
            if was_raining {
                world.broadcast_packet_all(&CGameEvent::new(GameEvent::EndRaining, 0.0));
            } else {
                world.broadcast_packet_all(&CGameEvent::new(GameEvent::BeginRaining, 0.0));
            }
        }
    }

    pub fn tick_weather(&mut self, world: &World) {
        if self.weather_cycle_enabled {
            self.advance_weather_cycle();
        }

        // Update visual transitions
        self.old_rain_level = self.rain_level;
        self.old_thunder_level = self.thunder_level;

        if self.raining {
            self.rain_level = (self.rain_level + WEATHER_TRANSITION_SPEED).min(1.0);
        } else {
            self.rain_level = (self.rain_level - WEATHER_TRANSITION_SPEED).max(0.0);
        }

        if self.thundering {
            self.thunder_level = (self.thunder_level + WEATHER_TRANSITION_SPEED).min(1.0);
        } else {
            self.thunder_level = (self.thunder_level - WEATHER_TRANSITION_SPEED).max(0.0);
        }

        // Broadcast level changes if needed
        if (self.old_rain_level - self.rain_level).abs() > f32::EPSILON {
            world.broadcast_packet_all(&CGameEvent::new(
                GameEvent::RainLevelChange,
                self.rain_level,
            ));
        }

        if (self.old_thunder_level - self.thunder_level).abs() > f32::EPSILON {
            world.broadcast_packet_all(&CGameEvent::new(
                GameEvent::ThunderLevelChange,
                self.thunder_level,
            ));
        }
    }

    fn advance_weather_cycle(&mut self) {
        if self.clear_weather_time > 0 {
            self.clear_weather_time -= 1;
            // Match ServerLevel: a state which was already clear receives a
            // one-tick timer, while an active state is held at zero.
            self.thunder_time = i32::from(!self.thundering);
            self.rain_time = i32::from(!self.raining);
            self.thundering = false;
            self.raining = false;
        } else {
            // Handle thunder timing
            if self.thunder_time > 0 {
                self.thunder_time -= 1;
                if self.thunder_time == 0 {
                    self.thundering = !self.thundering;
                }
            } else if self.thundering {
                self.thunder_time =
                    rand::rng().random_range(THUNDER_DURATION_MIN..=THUNDER_DURATION_MAX);
            } else {
                self.thunder_time = rand::rng().random_range(THUNDER_DELAY_MIN..=THUNDER_DELAY_MAX);
            }

            // Handle rain timing
            if self.rain_time > 0 {
                self.rain_time -= 1;
                if self.rain_time == 0 {
                    self.raining = !self.raining;
                }
            } else if self.raining {
                self.rain_time = rand::rng().random_range(RAIN_DURATION_MIN..=RAIN_DURATION_MAX);
            } else {
                self.rain_time = rand::rng().random_range(RAIN_DELAY_MIN..=RAIN_DELAY_MAX);
            }
        }
    }

    pub fn reset_weather_cycle(&mut self, world: &World) {
        self.set_weather_parameters(world, 0, 0, false, false);
    }
}

impl Clone for Weather {
    fn clone(&self) -> Self {
        Self {
            clear_weather_time: self.clear_weather_time,
            raining: self.raining,
            rain_time: self.rain_time,
            thundering: self.thundering,
            thunder_time: self.thunder_time,
            rain_level: self.rain_level,
            old_rain_level: self.old_rain_level,
            thunder_level: self.thunder_level,
            old_thunder_level: self.old_thunder_level,
            weather_cycle_enabled: self.weather_cycle_enabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Weather;
    use pumpkin_world::world_info::data_files::WeatherData;

    #[test]
    fn weather_data_round_trip_preserves_all_vanilla_fields() {
        let data = WeatherData {
            rain_time: 123,
            raining: true,
            thundering: true,
            thunder_time: 456,
            clear_weather_time: 789,
            data_version: 4903,
        };
        let weather = Weather::from_data(&data);
        let restored = weather.to_data();
        assert_eq!(restored.rain_time, data.rain_time);
        assert_eq!(restored.raining, data.raining);
        assert_eq!(restored.thundering, data.thundering);
        assert_eq!(restored.thunder_time, data.thunder_time);
        assert_eq!(restored.clear_weather_time, data.clear_weather_time);
    }

    #[test]
    fn forced_clear_keeps_vanilla_timer_semantics() {
        let mut weather = Weather {
            clear_weather_time: 2,
            raining: true,
            rain_time: 40,
            thundering: false,
            thunder_time: 40,
            ..Weather::new()
        };
        // Calling the private state transition directly keeps this test
        // independent of a network-backed World.
        weather.advance_weather_cycle();
        assert_eq!(weather.clear_weather_time, 1);
        assert_eq!(weather.rain_time, 0);
        assert_eq!(weather.thunder_time, 1);
        assert!(!weather.raining);
        assert!(!weather.thundering);
    }
}
