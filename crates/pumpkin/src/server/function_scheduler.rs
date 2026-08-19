//! Deterministic queue used by `/schedule function`.
//!
//! Vanilla's `TimerQueue` orders by trigger time and an insertion sequence,
//! replaces an event only when the same `(id, trigger_time)` already exists,
//! and removes every trigger carrying an id for `/schedule clear`.  Keeping
//! those rules in a small synchronous type makes the async server integration
//! unable to reorder callbacks while locks are released for command execution.

use std::cmp::Ordering;

use pumpkin_world::world_info::data_files::ScheduledFunctionData;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledFunction {
    pub trigger_time: i64,
    pub id: String,
    pub tag: bool,
    sequence: u64,
}

#[derive(Clone, Debug, Default)]
pub struct FunctionScheduler {
    events: Vec<ScheduledFunction>,
    next_sequence: u64,
}

impl FunctionScheduler {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            events: Vec::new(),
            next_sequence: 0,
        }
    }

    pub fn load(&mut self, events: impl IntoIterator<Item = ScheduledFunctionData>) {
        self.events.clear();
        self.next_sequence = 0;
        for event in events {
            self.push(event.trigger_time, event.id, event.tag);
        }
    }

    /// Adds an event. `replace` removes the same callback id first; append
    /// keeps older callbacks and still de-duplicates an identical trigger.
    pub fn schedule(&mut self, id: String, trigger_time: i64, tag: bool, replace: bool) -> bool {
        if replace {
            self.events
                .retain(|event| event.id != id || event.tag != tag);
        }
        if self
            .events
            .iter()
            .any(|event| event.id == id && event.tag == tag && event.trigger_time == trigger_time)
        {
            return false;
        }
        self.push(trigger_time, id, tag);
        true
    }

    pub fn clear(&mut self, id: &str, tag: Option<bool>) -> usize {
        let before = self.events.len();
        self.events
            .retain(|event| event.id != id || tag.is_some_and(|expected| event.tag != expected));
        before.saturating_sub(self.events.len())
    }

    #[must_use]
    pub fn due(&mut self, now: i64) -> Vec<ScheduledFunction> {
        let mut due = Vec::new();
        let mut pending = Vec::with_capacity(self.events.len());
        for event in self.events.drain(..) {
            if event.trigger_time <= now {
                due.push(event);
            } else {
                pending.push(event);
            }
        }
        self.events = pending;
        due.sort_by(compare_events);
        due
    }

    #[must_use]
    pub fn snapshot(&self) -> Vec<ScheduledFunctionData> {
        let mut ordered = self.events.iter().collect::<Vec<_>>();
        ordered.sort_by(|a, b| compare_events(a, b));
        ordered
            .into_iter()
            .map(|event| ScheduledFunctionData {
                trigger_time: event.trigger_time,
                id: event.id.clone(),
                tag: event.tag,
            })
            .collect()
    }

    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        let mut ids = self
            .events
            .iter()
            .map(|event| {
                if event.tag {
                    format!("#{}", event.id)
                } else {
                    event.id.clone()
                }
            })
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        ids
    }

    fn push(&mut self, trigger_time: i64, id: String, tag: bool) {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.events.push(ScheduledFunction {
            trigger_time,
            id,
            tag,
            sequence,
        });
    }
}

fn compare_events(a: &ScheduledFunction, b: &ScheduledFunction) -> Ordering {
    a.trigger_time
        .cmp(&b.trigger_time)
        .then_with(|| a.sequence.cmp(&b.sequence))
}

#[cfg(test)]
mod tests {
    use super::FunctionScheduler;

    #[test]
    fn replace_append_clear_and_due_follow_timer_queue_rules() {
        let mut queue = FunctionScheduler::new();
        assert!(queue.schedule("example:a".to_owned(), 20, false, true));
        assert!(queue.schedule("example:a".to_owned(), 10, false, false));
        assert!(!queue.schedule("example:a".to_owned(), 10, false, false));
        assert!(queue.schedule("example:b".to_owned(), 10, false, true));
        assert_eq!(
            queue.due(10).into_iter().map(|e| e.id).collect::<Vec<_>>(),
            ["example:a", "example:b"]
        );
        assert_eq!(queue.clear("example:a", None), 1);
        assert!(queue.ids().is_empty());
    }

    #[test]
    fn tag_ids_are_distinct_from_function_ids_for_clear_and_listing() {
        let mut queue = FunctionScheduler::new();
        queue.schedule("example:tick".to_owned(), 5, true, true);
        queue.schedule("example:tick".to_owned(), 6, false, true);
        assert_eq!(queue.ids(), vec!["#example:tick", "example:tick"]);
        assert_eq!(queue.clear("example:tick", Some(true)), 1);
        assert_eq!(queue.ids(), vec!["example:tick"]);
    }
}
