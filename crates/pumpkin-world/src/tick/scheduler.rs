use std::collections::BTreeMap;
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

use pumpkin_util::math::position::BlockPos;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::tick::{OrderedTick, ScheduledTick};

pub struct ChunkTickScheduler<T> {
    inner: Mutex<Option<Box<ChunkTickSchedulerInner<T>>>>,
    /// The next game-time value returned by `step_tick`.
    current_tick: AtomicU64,
}

struct ChunkTickSchedulerInner<T> {
    /// Absolute due time, rather than a bounded ring slot.  Vanilla stores an
    /// arbitrary integer delay in Anvil; a ring of 256 slots silently wraps a
    /// delay of 256+ into an earlier tick.
    tick_queue: BTreeMap<u64, Vec<OrderedTick<T>>>,
    queued_ticks: FxHashSet<(BlockPos, T)>,
    inflight_ticks: FxHashMap<(BlockPos, T), OrderedTick<T>>,
}

impl<'a, T: std::hash::Hash + Eq> ChunkTickScheduler<&'a T> {
    pub fn step_tick(&self) -> Vec<OrderedTick<&'a T>> {
        let current_tick = self.current_tick.fetch_add(1, Ordering::SeqCst);

        let mut inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(inner) = inner_guard.as_mut() else {
            return Vec::new();
        };

        // Vanilla's LevelTicks drains every entry whose trigger time is less
        // than or equal to the current game time.  A tick scheduled with
        // delay zero from inside another callback therefore runs on the next
        // server tick instead of becoming stranded in an already-consumed
        // exact-time bucket.  Draining a range also recovers safely if the
        // server was paused and more than one game time elapsed between
        // scheduler steps.
        let due_times: Vec<u64> = inner
            .tick_queue
            .range(..=current_tick)
            .map(|(due_tick, _)| *due_tick)
            .collect();
        let mut res = Vec::new();
        for due_tick in due_times {
            if let Some(mut ticks) = inner.tick_queue.remove(&due_tick) {
                res.append(&mut ticks);
            }
        }

        if !res.is_empty() {
            for next_tick in &res {
                inner
                    .queued_ticks
                    .remove(&(next_tick.position, next_tick.value));
                inner
                    .inflight_ticks
                    .insert((next_tick.position, next_tick.value), next_tick.clone());
            }
        }
        res
    }

    pub fn schedule_tick(&self, tick: &ScheduledTick<&'a T>, sub_tick_order: u64) {
        let mut inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let inner = inner_guard.get_or_insert_with(|| {
            Box::new(ChunkTickSchedulerInner {
                tick_queue: BTreeMap::new(),
                queued_ticks: FxHashSet::default(),
                inflight_ticks: FxHashMap::default(),
            })
        });

        if inner.queued_ticks.insert((tick.position, tick.value)) {
            let due_tick = self
                .current_tick
                .load(Ordering::SeqCst)
                .saturating_add(u64::from(tick.delay));

            inner
                .tick_queue
                .entry(due_tick)
                .or_default()
                .push(OrderedTick {
                    priority: tick.priority,
                    sub_tick_order,
                    position: tick.position,
                    value: tick.value,
                });
        }
    }

    pub fn is_scheduled(&self, pos: BlockPos, value: &T) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|inner| {
                inner.queued_ticks.contains(&(pos, value))
                    || inner.inflight_ticks.contains_key(&(pos, value))
            })
    }

    pub fn has_ticks(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|inner| !inner.queued_ticks.is_empty() || !inner.inflight_ticks.is_empty())
    }

    pub fn clear_inflight(&self) {
        let mut inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(inner) = inner_guard.as_mut() {
            inner.inflight_ticks.clear();
            if inner.queued_ticks.is_empty() && inner.inflight_ticks.is_empty() {
                *inner_guard = None;
            }
        }
    }

    #[must_use]
    pub fn to_vec(&self) -> Vec<ScheduledTick<&'a T>> {
        let current_tick = self.current_tick.load(Ordering::SeqCst);
        let inner_guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(inner) = inner_guard.as_ref() else {
            return Vec::new();
        };

        let mut res = Vec::new();

        for (due_tick, ticks) in &inner.tick_queue {
            let delay = due_tick
                .saturating_sub(current_tick)
                .min(u64::from(u32::MAX)) as u32;
            res.extend(ticks.iter().map(|x| ScheduledTick {
                delay,
                priority: x.priority,
                position: x.position,
                value: x.value,
            }));
        }
        // A chunk can be serialized while a due tick is being processed by
        // the authoritative world loop. Persist that due work at delay zero;
        // otherwise a shutdown in that small window loses the update.
        res.extend(inner.inflight_ticks.values().map(|tick| ScheduledTick {
            delay: 0,
            priority: tick.priority,
            position: tick.position,
            value: tick.value,
        }));
        res
    }
}

impl<'a, T: std::hash::Hash + Eq + 'static> FromIterator<ScheduledTick<&'a T>>
    for ChunkTickScheduler<&'a T>
{
    fn from_iter<I: IntoIterator<Item = ScheduledTick<&'a T>>>(iter: I) -> Self {
        let scheduler = Self::default();
        let iter = iter.into_iter();

        let (lower, _) = iter.size_hint();
        if lower > 0 {
            let mut inner_guard = scheduler
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let inner = inner_guard.get_or_insert_with(|| {
                Box::new(ChunkTickSchedulerInner {
                    tick_queue: BTreeMap::new(),
                    queued_ticks: FxHashSet::default(),
                    inflight_ticks: FxHashMap::default(),
                })
            });
            inner.queued_ticks.reserve(lower);
        }

        // The Anvil list is ordered by the vanilla scheduler's insertion
        // sequence. Preserve that sequence when rebuilding the absolute-time queue after a
        // load; assigning zero to every tick makes equal-priority redstone
        // updates depend on `sort_unstable`'s allocation order.
        for (sub_tick_order, tick) in iter.enumerate() {
            scheduler.schedule_tick(&tick, sub_tick_order as u64);
        }
        scheduler
    }
}

impl<T> Default for ChunkTickScheduler<T> {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
            current_tick: AtomicU64::new(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tick::TickPriority;
    use pumpkin_util::math::position::BlockPos;

    static VALUE: u8 = 0;

    #[test]
    fn inflight_tick_still_appears_scheduled() {
        let scheduler: ChunkTickScheduler<&'static u8> = ChunkTickScheduler::default();
        let pos = BlockPos::new(0, 0, 0);

        scheduler.schedule_tick(
            &ScheduledTick {
                delay: 0,
                priority: TickPriority::Normal,
                position: pos,
                value: &VALUE,
            },
            0,
        );

        assert!(scheduler.is_scheduled(pos, &VALUE));

        let ticks = scheduler.step_tick();
        assert_eq!(ticks.len(), 1);
        assert_eq!(ticks[0].position, pos);

        // Tick is in-flight — is_scheduled must still return true
        assert!(scheduler.is_scheduled(pos, &VALUE));

        // Scheduling a fresh tick for the same position must succeed
        // while the old one is in-flight
        scheduler.schedule_tick(
            &ScheduledTick {
                delay: 5,
                priority: TickPriority::Normal,
                position: pos,
                value: &VALUE,
            },
            1,
        );

        scheduler.clear_inflight();

        // After clear, in-flight is gone but the fresh queued tick remains
        assert!(scheduler.is_scheduled(pos, &VALUE));
        assert!(scheduler.has_ticks());

        // Step again to retrieve the fresh tick
        for _ in 0..5 {
            scheduler.step_tick();
        }
        let ticks = scheduler.step_tick();
        assert_eq!(ticks.len(), 1);
        assert_eq!(ticks[0].position, pos);
    }

    #[test]
    fn clear_inflight_drops_inner_when_empty() {
        let scheduler: ChunkTickScheduler<&'static u8> = ChunkTickScheduler::default();
        let pos = BlockPos::new(0, 0, 0);

        scheduler.schedule_tick(
            &ScheduledTick {
                delay: 0,
                priority: TickPriority::Normal,
                position: pos,
                value: &VALUE,
            },
            0,
        );

        let _ticks = scheduler.step_tick();
        scheduler.clear_inflight();

        assert!(!scheduler.has_ticks());
    }

    #[test]
    fn loaded_ticks_keep_input_order_for_equal_priority() {
        let first = BlockPos::new(1, 0, 0);
        let second = BlockPos::new(2, 0, 0);
        let ticks = vec![
            ScheduledTick {
                delay: 0,
                priority: TickPriority::Normal,
                position: first,
                value: &VALUE,
            },
            ScheduledTick {
                delay: 0,
                priority: TickPriority::Normal,
                position: second,
                value: &VALUE,
            },
        ];
        let scheduler: ChunkTickScheduler<&'static u8> = ticks.into_iter().collect();
        let mut ready = scheduler.step_tick();
        ready.sort_unstable();
        assert_eq!(ready[0].position, first);
        assert_eq!(ready[1].position, second);
    }

    #[test]
    fn long_delays_do_not_wrap_through_a_256_tick_wheel() {
        let scheduler: ChunkTickScheduler<&'static u8> = ChunkTickScheduler::default();
        let pos = BlockPos::new(3, 0, 0);
        scheduler.schedule_tick(
            &ScheduledTick {
                delay: 1_000,
                priority: TickPriority::Normal,
                position: pos,
                value: &VALUE,
            },
            0,
        );

        assert_eq!(scheduler.to_vec()[0].delay, 1_000);
        for _ in 0..1_000 {
            assert!(scheduler.step_tick().is_empty());
        }
        assert_eq!(scheduler.step_tick().len(), 1);
    }

    #[test]
    fn overdue_zero_delay_tick_runs_on_the_next_step() {
        let scheduler: ChunkTickScheduler<&'static u8> = ChunkTickScheduler::default();
        let first_pos = BlockPos::new(4, 0, 0);
        let second_pos = BlockPos::new(5, 0, 0);

        scheduler.schedule_tick(
            &ScheduledTick {
                delay: 0,
                priority: TickPriority::Normal,
                position: first_pos,
                value: &VALUE,
            },
            0,
        );
        assert_eq!(scheduler.step_tick().len(), 1);

        // This models a block callback scheduling another zero-delay update
        // after the current tick's due bucket has already been drained.
        scheduler.schedule_tick(
            &ScheduledTick {
                delay: 0,
                priority: TickPriority::Normal,
                position: second_pos,
                value: &VALUE,
            },
            1,
        );
        let next = scheduler.step_tick();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].position, second_pos);
    }
}
