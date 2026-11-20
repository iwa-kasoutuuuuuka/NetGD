//! Sequential read-ahead prediction.
//!
//! The firmware stages each `CD_READ` through one buffer and keeps the last
//! range it staged. After a request has been completed the buffer is idle
//! until the host's next command, so if the host is streaming (each request
//! starts where the previous one ended) the sectors it will ask for next can
//! be fetched in that idle time and the next request's first chunk becomes a
//! cache hit instead of a card access.
//!
//! This module only decides *whether* and *what* to fetch. Fetching is the
//! firmware's job, and so are the two limits only it knows: how many sectors
//! one staging round holds and how many are left in the track.

/// A range of disc sectors to fetch ahead of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prefetch {
    pub lba: u32,
    pub count: u32,
}

impl Prefetch {
    /// Limit to what one staging round holds (`stage_max`) and to what is
    /// left of the track (`remaining`). `None` if nothing is left.
    pub fn clamped(self, stage_max: u32, remaining: u32) -> Option<Prefetch> {
        let count = self.count.min(stage_max).min(remaining);
        if count == 0 {
            None
        } else {
            Some(Prefetch { lba: self.lba, count })
        }
    }
}

/// Remembers where the previous request ended and how big it was.
#[derive(Clone, Copy, Debug)]
pub struct ReadAhead {
    /// First sector after the previous request.
    end: Option<u32>,
    /// Sectors the next request's first chunk is expected to want.
    size: u32,
}

impl ReadAhead {
    pub const fn new() -> Self {
        ReadAhead { end: None, size: 0 }
    }

    /// Forget the history (eject, load, reset).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Record a completed request of `count` sectors at `lba`, where
    /// `first_chunk_max` is how many sectors the first staging round of that
    /// request could hold. Returns the range to fetch ahead *if the request
    /// continued the previous one*, i.e. the host looks like it is streaming;
    /// a single isolated read, or a seek, returns `None` so random access
    /// does not pay for fetches nobody uses.
    ///
    /// The caller still has to [`Prefetch::clamped`] the result: the range
    /// may start in another track with a different staging size.
    pub fn record(&mut self, lba: u32, count: u32, first_chunk_max: u32) -> Option<Prefetch> {
        let sequential = self.end == Some(lba);
        self.end = lba.checked_add(count);
        // The next request is predicted to look like this one, and only its
        // first chunk can be a hit, so that is the size worth fetching.
        self.size = count.min(first_chunk_max);
        match self.end {
            Some(end) if sequential && self.size != 0 => {
                Some(Prefetch { lba: end, count: self.size })
            }
            _ => None,
        }
    }
}

impl Default for ReadAhead {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_request_never_prefetches() {
        let mut r = ReadAhead::new();
        assert_eq!(r.record(100, 16, 7), None);
    }

    #[test]
    fn sequential_request_prefetches_the_continuation() {
        let mut r = ReadAhead::new();
        assert_eq!(r.record(100, 4, 7), None);
        // 104.. continues 100..104.
        assert_eq!(r.record(104, 4, 7), Some(Prefetch { lba: 108, count: 4 }));
        assert_eq!(r.record(108, 4, 7), Some(Prefetch { lba: 112, count: 4 }));
    }

    #[test]
    fn seek_breaks_the_streak() {
        let mut r = ReadAhead::new();
        r.record(100, 4, 7);
        assert!(r.record(104, 4, 7).is_some());
        assert_eq!(r.record(5000, 4, 7), None);
        // ... and the streak must be re-established.
        assert_eq!(r.record(5004, 4, 7), Some(Prefetch { lba: 5008, count: 4 }));
    }

    #[test]
    fn gap_or_overlap_is_not_sequential() {
        let mut r = ReadAhead::new();
        r.record(100, 4, 7);
        assert_eq!(r.record(105, 4, 7), None);
        assert_eq!(r.record(106, 4, 7), None); // overlaps the previous request
    }

    #[test]
    fn size_follows_the_last_request_capped_at_the_first_chunk() {
        let mut r = ReadAhead::new();
        r.record(0, 572, 7);
        // 572 sectors continue at 572; only a first chunk can be a hit.
        assert_eq!(r.record(572, 572, 7), Some(Prefetch { lba: 1144, count: 7 }));
        assert_eq!(r.record(1144, 2, 7), Some(Prefetch { lba: 1146, count: 2 }));
    }

    #[test]
    fn zero_count_request_prefetches_nothing() {
        let mut r = ReadAhead::new();
        r.record(100, 4, 7);
        assert_eq!(r.record(104, 0, 7), None);
    }

    #[test]
    fn address_overflow_is_not_sequential() {
        let mut r = ReadAhead::new();
        assert_eq!(r.record(u32::MAX - 1, 4, 7), None);
        assert_eq!(r.record(u32::MAX, 1, 7), None);
    }

    #[test]
    fn reset_forgets() {
        let mut r = ReadAhead::new();
        r.record(100, 4, 7);
        r.reset();
        assert_eq!(r.record(104, 4, 7), None);
    }

    #[test]
    fn clamp_to_stage_and_track() {
        let p = Prefetch { lba: 10, count: 9 };
        assert_eq!(p.clamped(7, 100), Some(Prefetch { lba: 10, count: 7 }));
        assert_eq!(p.clamped(7, 3), Some(Prefetch { lba: 10, count: 3 }));
        assert_eq!(p.clamped(7, 0), None);
        assert_eq!(p.clamped(0, 100), None);
    }
}
