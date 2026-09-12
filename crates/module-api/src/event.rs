//! Sample-accurate parameter events (ADR-005 §4).

use crate::param::ParamId;

/// A parameter change at a sample offset within one `process()` block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamEvent {
    /// Offset within the block: `0 <= offset < frames`; all offsets are 0 when `frames == 0`.
    pub offset: u32,
    /// Target parameter.
    pub id: ParamId,
    /// New plain value, already `clamp_quantize`d by the host.
    pub value: f64,
}

/// Default capacity of per-slot event lists (ADR-002 §3).
pub const DEFAULT_EVENT_CAPACITY: usize = 512;

/// Error pushing into an [`EventList`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EventListError {
    /// The list is at capacity.
    #[error("event list is full")]
    Full,
    /// The event's offset is before the last pushed offset.
    #[error("event offset is before the previous event's offset")]
    OutOfOrder,
}

/// Host-side builder of a block's time-sorted events. Fixed capacity (allocated off the audio
/// thread); never reallocates. All methods except the constructor are RT-safe.
#[derive(Debug)]
pub struct EventList {
    events: Vec<ParamEvent>,
    capacity: usize,
}

/// Allocates the full capacity (a derived `Clone` keeps only `len`), so a clone never
/// reallocates either. \[control thread\]
impl Clone for EventList {
    fn clone(&self) -> Self {
        let mut events = Vec::with_capacity(self.capacity);
        events.extend_from_slice(&self.events);
        Self {
            events,
            capacity: self.capacity,
        }
    }
}

impl EventList {
    /// Allocates a list holding at most `capacity` events. \[control thread\]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            events: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Removes all events (keeps the allocation).
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// Appends an event. `Err(Full)` at capacity; `Err(OutOfOrder)` if `e.offset` is smaller
    /// than the last pushed offset.
    pub fn push(&mut self, e: ParamEvent) -> Result<(), EventListError> {
        if self.events.len() >= self.capacity {
            return Err(EventListError::Full);
        }
        if self
            .events
            .last()
            .is_some_and(|last| e.offset < last.offset)
        {
            return Err(EventListError::OutOfOrder);
        }
        self.events.push(e);
        Ok(())
    }

    /// The events, sorted by offset.
    pub fn as_slice(&self) -> &[ParamEvent] {
        &self.events
    }

    /// Number of events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True if there are no events.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Maximum number of events.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Block splitting: **appends** the events with offset in `[start, start + len)` to `out`,
    /// rebased so `start` becomes offset 0. `out` is not cleared, so carried-over events can be
    /// pushed first. On `Err(Full)` the events copied so far stay in `out`.
    pub fn split_into(
        &self,
        start: u32,
        len: u32,
        out: &mut EventList,
    ) -> Result<(), EventListError> {
        let end = start.saturating_add(len);
        for e in self
            .events
            .iter()
            .filter(|e| e.offset >= start && e.offset < end)
        {
            out.push(ParamEvent {
                offset: e.offset - start,
                ..*e
            })?;
        }
        Ok(())
    }
}

/// Module → host parameter reports (READ_ONLY params; adapter-originated changes).
/// Fixed capacity; never reallocates. All methods except the constructor are RT-safe.
#[derive(Debug)]
pub struct OutputEvents {
    events: Vec<ParamEvent>,
    capacity: usize,
}

/// Allocates the full capacity (a derived `Clone` keeps only `len`), so a clone never
/// reallocates either. \[control thread\]
impl Clone for OutputEvents {
    fn clone(&self) -> Self {
        let mut events = Vec::with_capacity(self.capacity);
        events.extend_from_slice(&self.events);
        Self {
            events,
            capacity: self.capacity,
        }
    }
}

impl OutputEvents {
    /// Allocates room for `capacity` reports. \[control thread\]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            events: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Adds a report; gives the event back when full.
    pub fn try_push(&mut self, e: ParamEvent) -> Result<(), ParamEvent> {
        if self.events.len() >= self.capacity {
            return Err(e);
        }
        self.events.push(e);
        Ok(())
    }

    /// Reports in push order.
    pub fn as_slice(&self) -> &[ParamEvent] {
        &self.events
    }

    /// Removes all reports (keeps the allocation). The host calls it before each block.
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// Number of reports.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True if there are no reports.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// One piece of a block between event offsets: apply `events` first, then render
/// `start..start + len`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment<'a> {
    /// First sample of the segment.
    pub start: u32,
    /// Number of samples to render (0 only for a parameter flush or trailing events).
    pub len: u32,
    /// Events to apply before rendering (all events whose offset is `start`).
    pub events: &'a [ParamEvent],
}

/// Splits `0..frames` at event offsets. Each segment first applies its `events` (all events
/// whose offset == start), then renders `len` samples. For `frames == 0`: one segment
/// `(0, 0, all events)`. RT-safe (no allocation, bounded by `frames` and `events.len()`).
///
/// Robustness for invalid input (never produced by the host): events whose offset is before the
/// current position are applied at the current segment; events at or past `frames` are returned
/// in a final zero-length segment at `frames`, so no event is ever skipped.
pub fn segments(frames: u32, events: &[ParamEvent]) -> impl Iterator<Item = Segment<'_>> {
    Segments {
        frames,
        events,
        pos: 0,
        idx: 0,
        done: false,
    }
}

struct Segments<'a> {
    frames: u32,
    events: &'a [ParamEvent],
    pos: u32,
    idx: usize,
    done: bool,
}

impl<'a> Iterator for Segments<'a> {
    type Item = Segment<'a>;

    fn next(&mut self) -> Option<Segment<'a>> {
        if self.done {
            return None;
        }
        if self.frames == 0 {
            self.done = true;
            return Some(Segment {
                start: 0,
                len: 0,
                events: self.events,
            });
        }
        if self.pos >= self.frames {
            self.done = true;
            let rest = &self.events[self.idx..];
            return (!rest.is_empty()).then_some(Segment {
                start: self.frames,
                len: 0,
                events: rest,
            });
        }
        let start = self.pos;
        let mut j = self.idx;
        while j < self.events.len() && self.events[j].offset <= start {
            j += 1;
        }
        let end = self
            .events
            .get(j)
            .map_or(self.frames, |e| e.offset.min(self.frames));
        let seg = Segment {
            start,
            len: end - start,
            events: &self.events[self.idx..j],
        };
        self.idx = j;
        self.pos = end;
        Some(seg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(offset: u32, id: u32) -> ParamEvent {
        ParamEvent {
            offset,
            id: ParamId(id),
            value: f64::from(id),
        }
    }

    fn collect(frames: u32, events: &[ParamEvent]) -> Vec<(u32, u32, Vec<u32>)> {
        segments(frames, events)
            .map(|s| (s.start, s.len, s.events.iter().map(|e| e.id.0).collect()))
            .collect()
    }

    #[test]
    fn event_list_capacity_and_order() {
        let mut l = EventList::with_capacity(2);
        assert!(l.is_empty());
        l.push(ev(3, 0)).unwrap();
        assert_eq!(l.push(ev(2, 1)), Err(EventListError::OutOfOrder));
        l.push(ev(3, 1)).unwrap();
        assert_eq!(l.push(ev(4, 2)), Err(EventListError::Full));
        assert_eq!(l.len(), 2);
        l.clear();
        assert!(l.is_empty());
        assert_eq!(l.capacity(), 2);
    }

    #[test]
    fn split_into_rebases_and_appends() {
        let mut l = EventList::with_capacity(8);
        for (o, id) in [(0, 0), (10, 1), (63, 2), (64, 3), (100, 4)] {
            l.push(ev(o, id)).unwrap();
        }
        let mut out = EventList::with_capacity(8);
        l.split_into(0, 64, &mut out).unwrap();
        assert_eq!(out.as_slice(), &[ev(0, 0), ev(10, 1), ev(63, 2)]);
        out.clear();
        out.push(ev(0, 9)).unwrap();
        l.split_into(64, 64, &mut out).unwrap();
        let got: Vec<_> = out.as_slice().iter().map(|e| (e.offset, e.id.0)).collect();
        assert_eq!(got, vec![(0, 9), (0, 3), (36, 4)]);
        let mut small = EventList::with_capacity(1);
        assert_eq!(l.split_into(0, 64, &mut small), Err(EventListError::Full));
        assert_eq!(small.len(), 1);
    }

    #[test]
    fn output_events_give_back_when_full() {
        let mut o = OutputEvents::with_capacity(1);
        o.try_push(ev(0, 0)).unwrap();
        assert_eq!(o.try_push(ev(0, 1)), Err(ev(0, 1)));
        assert_eq!(o.len(), 1);
        o.clear();
        assert!(o.is_empty());
    }

    #[test]
    fn segments_split_at_offsets() {
        assert_eq!(collect(8, &[]), vec![(0, 8, vec![])]);
        assert_eq!(
            collect(8, &[ev(0, 1), ev(3, 2), ev(3, 3), ev(7, 4)]),
            vec![(0, 3, vec![1]), (3, 4, vec![2, 3]), (7, 1, vec![4])]
        );
        assert_eq!(
            collect(8, &[ev(5, 1)]),
            vec![(0, 5, vec![]), (5, 3, vec![1])]
        );
    }

    #[test]
    fn segments_zero_frames_is_a_flush() {
        assert_eq!(collect(0, &[]), vec![(0, 0, vec![])]);
        assert_eq!(collect(0, &[ev(0, 1), ev(0, 2)]), vec![(0, 0, vec![1, 2])]);
    }

    #[test]
    fn segments_never_skip_invalid_events() {
        // Offset past the block: applied in a trailing zero-length segment.
        assert_eq!(
            collect(4, &[ev(9, 1)]),
            vec![(0, 4, vec![]), (4, 0, vec![1])]
        );
        // Unsorted: the late event is applied at the current segment.
        assert_eq!(
            collect(8, &[ev(4, 1), ev(2, 2)]),
            vec![(0, 4, vec![]), (4, 4, vec![1, 2])]
        );
        let total: u32 = segments(1000, &[ev(1, 0), ev(999, 1)]).map(|s| s.len).sum();
        assert_eq!(total, 1000);
    }
}
