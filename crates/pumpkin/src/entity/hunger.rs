use std::sync::Arc;

use super::{NBTStorage, NBTStorageInit, player::Player};
use crate::entity::NbtFuture;
use crossbeam::atomic::AtomicCell;
use pumpkin_data::damage::DamageType;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::Difficulty;

const MAX_FOOD: u8 = 20;
const EXHAUSTION_COST: f32 = 4.0;
const MAX_EXHAUSTION: f32 = 40.0;

#[must_use]
fn food_level_after_eating(current: u8, food: u8) -> u8 {
    // Item food values are network/NBT data and may be 255 when supplied by a
    // datapack.  Vanilla's integer math is bounded by the 20-point food bar;
    // a wrapping u8 addition would otherwise turn a full player back to zero.
    current.saturating_add(food).min(MAX_FOOD)
}

#[must_use]
fn bounded_saturation(value: f32, max: f32) -> f32 {
    if !value.is_nan() {
        value.clamp(0.0, max.max(0.0))
    } else {
        0.0
    }
}

#[must_use]
fn bounded_exhaustion(value: f32) -> f32 {
    if !value.is_nan() {
        value.clamp(0.0, MAX_EXHAUSTION)
    } else {
        0.0
    }
}

pub struct HungerManager {
    pub level: AtomicCell<u8>,
    pub saturation: AtomicCell<f32>,
    pub exhaustion: AtomicCell<f32>,
    pub tick_timer: AtomicCell<u32>,
}

impl Default for HungerManager {
    fn default() -> Self {
        Self {
            level: AtomicCell::new(MAX_FOOD),
            saturation: AtomicCell::new(5.0),
            exhaustion: AtomicCell::new(0.0),
            tick_timer: AtomicCell::new(0),
        }
    }
}

impl HungerManager {
    pub async fn tick(&self, player: &Arc<Player>) {
        let mut level = self.level.load();
        let mut saturation = self.saturation.load();
        let mut exhaustion = self.exhaustion.load();
        let mut timer = self.tick_timer.load();

        let level_info = player.world().level_info.load();
        let difficulty = level_info.difficulty;
        let natural_regen = level_info.game_rules.natural_health_regeneration;
        let health = player.living_entity.health.load();
        let can_heal = player.can_food_heal();

        let mut needs_sync = false;
        let mut heal_amount = 0.0;
        let mut damage_amount = 0.0;

        if exhaustion > EXHAUSTION_COST {
            exhaustion -= EXHAUSTION_COST;
            if saturation > 0.0 {
                saturation = (saturation - 1.0).max(0.0);
            } else if difficulty != Difficulty::Peaceful {
                level = level.saturating_sub(1);
            }
            needs_sync = true;
        }

        if natural_regen && saturation > 0.0 && can_heal && level >= 20 {
            timer += 1;
            if timer >= 10 {
                let cost = saturation.min(6.0);
                saturation -= cost;
                exhaustion += cost;
                heal_amount = cost / 6.0;
                timer = 0;
                needs_sync = true;
            }
        } else if natural_regen && level >= 18 && can_heal {
            timer += 1;
            if timer >= 80 {
                heal_amount = 1.0;
                exhaustion += 6.0;
                timer = 0;
                needs_sync = true;
            }
        } else if level == 0 {
            timer += 1;
            if timer >= 80 {
                timer = 0;
                let should_starve = match difficulty {
                    Difficulty::Peaceful => false,
                    Difficulty::Easy => health > 10.0,
                    Difficulty::Normal => health > 1.0,
                    Difficulty::Hard => true,
                };

                if should_starve {
                    damage_amount = 1.0;
                }
                self.tick_timer.store(0);
            }
        } else {
            timer = 0;
        }

        if needs_sync || timer != self.tick_timer.load() {
            self.level.store(level);
            self.saturation.store(saturation);
            self.exhaustion.store(exhaustion);
            self.tick_timer.store(timer);
        }

        if needs_sync {
            player.send_health().await;
        }
        if heal_amount > 0.0 {
            player.heal(heal_amount).await;
        }
        if damage_amount > 0.0 {
            player
                .damage(&**player, damage_amount, DamageType::STARVE)
                .await;
        }
    }

    pub async fn eat(&self, player: &Player, food: u8, saturation_modifier: f32) {
        let added_saturation = f32::from(food) * saturation_modifier * 2.0;

        let current_level = self.level.load();
        let current_sat = self.saturation.load();

        let new_level = food_level_after_eating(current_level, food);

        let new_sat = bounded_saturation(current_sat + added_saturation, f32::from(new_level));

        self.level.store(new_level);
        self.saturation.store(new_sat);

        player.send_health().await;
    }

    /// Add exhaustion to trigger hunger decrease
    pub fn add_exhaustion(&self, exhaustion: f32) {
        let current = self.exhaustion.load();
        self.exhaustion
            .store(bounded_exhaustion(current + exhaustion));
    }

    /// Add hunger manually
    pub fn add_hunger(&self, hunger: u8) {
        let current = self.level.load();
        self.level
            .store(current.saturating_add(hunger).min(MAX_FOOD));
    }

    /// Add saturation manually
    pub fn add_saturation(&self, saturation: f32) {
        let current = self.saturation.load();
        self.saturation.store(bounded_saturation(
            current + saturation,
            f32::from(self.level.load()),
        ));
    }

    pub fn set_level(&self, level: u8) {
        self.level.store(level.min(MAX_FOOD));
    }

    pub fn set_saturation(&self, saturation: f32) {
        self.saturation
            .store(bounded_saturation(saturation, f32::from(self.level.load())));
    }

    pub fn get_exhaustion(&self) -> f32 {
        self.exhaustion.load()
    }

    pub fn set_exhaustion(&self, exhaustion: f32) {
        self.exhaustion.store(bounded_exhaustion(exhaustion));
    }

    pub fn restart(&self) {
        self.level.store(MAX_FOOD);
        self.saturation.store(5.0);
        self.exhaustion.store(0.0);
        self.tick_timer.store(0);
    }
}

impl NBTStorage for HungerManager {
    fn write_nbt<'a>(&'a self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async {
            nbt.put_int("foodLevel", self.level.load().into());
            nbt.put_float("foodSaturationLevel", self.saturation.load());
            nbt.put_float("foodExhaustionLevel", self.exhaustion.load());
            nbt.put_int("foodTickTimer", self.tick_timer.load() as i32);
        })
    }

    fn read_nbt<'a>(&'a mut self, nbt: &'a mut NbtCompound) -> NbtFuture<'a, ()> {
        Box::pin(async move {
            let level = nbt
                .get_int("foodLevel")
                .and_then(|value| u8::try_from(value).ok())
                .unwrap_or(MAX_FOOD)
                .min(MAX_FOOD);
            self.level.store(level);
            self.saturation.store(bounded_saturation(
                nbt.get_float("foodSaturationLevel").unwrap_or(5.0),
                f32::from(level),
            ));
            self.exhaustion.store(bounded_exhaustion(
                nbt.get_float("foodExhaustionLevel").unwrap_or(0.0),
            ));
            let timer = nbt
                .get_int("foodTickTimer")
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(0);
            self.tick_timer.store(timer);
        })
    }
}

impl NBTStorageInit for HungerManager {}

#[cfg(test)]
mod tests {
    use super::{
        HungerManager, MAX_EXHAUSTION, bounded_exhaustion, bounded_saturation,
        food_level_after_eating,
    };

    #[test]
    fn food_bar_addition_is_saturating_for_datapack_values() {
        assert_eq!(food_level_after_eating(20, u8::MAX), 20);
        assert_eq!(food_level_after_eating(19, u8::MAX), 20);
        assert_eq!(food_level_after_eating(3, 4), 7);
    }

    #[test]
    fn malformed_food_floats_are_bounded() {
        assert_eq!(bounded_saturation(f32::NAN, 20.0), 0.0);
        assert_eq!(bounded_saturation(-2.0, 20.0), 0.0);
        assert_eq!(bounded_saturation(40.0, 20.0), 20.0);
        assert_eq!(bounded_exhaustion(f32::INFINITY), MAX_EXHAUSTION);
        assert_eq!(bounded_exhaustion(-1.0), 0.0);
        assert_eq!(bounded_exhaustion(100.0), MAX_EXHAUSTION);
    }

    #[test]
    fn manual_saturation_respects_current_food_level() {
        let manager = HungerManager::default();
        manager.level.store(2);
        manager.saturation.store(1.8);
        manager.add_saturation(0.4);
        assert_eq!(manager.saturation.load(), 2.0);
    }
}
