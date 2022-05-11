use crate::{LabelValues, Labels};
use num_traits::Zero;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// /!\ Magic number warning /!\
///
/// 4 is arbitrary. The code will function with even only one window, but any
/// new datapoints while sampling will be lost. Two is sufficient to capture
/// data while sampling the old window, simply bumping `current_window` so
/// samples can continue to be recorded. Four windows is only additionally
/// meaningful if we ever want to record actual samples of prior sampling
/// windows.
const SAMPLING_WINDOWS: usize = 4;

/// [`Windowing`] is a mechanism for rotating between different observations.
/// It provides an accessor [`Windowing::current`] for the current
/// observation, and a method [`Windowing::cycle_windows`] which makes the
/// next observation in the ring current, and returns the observation which
/// was current prior to the call.
pub struct Windowing<P> {
    current_window: AtomicUsize,
    windows: [Box<P>; SAMPLING_WINDOWS],
}

impl<P: Default> Windowing<P> {
    /// Constructor. Initializes its owned ring of `P`s using [`Default::default()`].
    pub fn new() -> Self {
        Self {
            current_window: AtomicUsize::new(0),
            windows: [
                Box::new(P::default()),
                Box::new(P::default()),
                Box::new(P::default()),
                Box::new(P::default()),
            ],
        }
    }

    /// Get the current collection. The underling `P` is expected to be
    /// cycled on some regular interval.
    ///
    /// Data integrity guarantees are weak. In some circumstances, the
    /// returned `P` window may be for the prior interval, if whatever wants
    /// to write a datapoint races with something replacing the current `P`.
    /// It is even possible (if extremely unlikely) for a value to be written
    /// into an old `P` collection, if the writer races with a reader and
    /// writes after the reader has emptied the collection and released its
    /// lock.
    pub fn current(&self) -> &P {
        &self.windows[self.current_window.load(Ordering::SeqCst)]
    }

    /// Cycle to the next window. Returns the window which was
    /// active before the call.
    pub fn cycle_windows(&self) -> &P {
        let old_idx = self.current_window.load(Ordering::SeqCst);
        self.current_window
            .store((old_idx + 1) % self.windows.len(), Ordering::SeqCst);
        &self.windows[old_idx]
    }
}

/// Since this is a constant shared for all ObservationSet, it currently must be tuned for the
/// busiest stat so as to not drop samples. An appropriate value for `WINDOW_SIZE` must be decided
/// in conjunction with the window sampling rate - currently at 15 seconds, this means the busiest
/// `ObservationSet` can handle ~4369 (65536 / 15) events per second.
const WINDOW_SIZE: usize = 65536;

struct ObservationSet<T: Ord + Zero + Copy> {
    idx: usize,
    wraps: usize,
    data: Box<[T]>,
}

impl<T: Ord + Zero + Copy> ObservationSet<T> {
    pub fn new() -> Self {
        Self {
            idx: 0,
            wraps: 0,
            // Construct in a manner that doesnt use stack space - Box::new([0; WINDOW_SIZE]) would
            data: vec![T::zero(); WINDOW_SIZE].into_boxed_slice(),
        }
    }

    /// Empty this ring buffer. The underlying data remains unchanged, but will be overwritten by
    /// at least `idx` entries when asking for the next sample, so it will not be visible in the
    /// future.
    fn clear(&mut self) {
        self.idx = 0;
        self.wraps = 0;
    }

    fn wraps(&self) -> usize {
        self.wraps
    }

    fn sorted_data(&mut self) -> &[T] {
        let data = &mut self.data[..self.idx];
        data.sort_unstable();
        data
    }

    fn add(&mut self, observation: T) {
        self.data[self.idx] = observation;

        self.idx = (self.idx + 1) % WINDOW_SIZE;
        if self.idx == 0 {
            // next_idx starts at 0, which means if we just added one and see zero, the index
            // wrapped.
            self.wraps = self.wraps.saturating_add(1);
        }
    }

    // While not currently used, `size` has a not-exactly-obvious implementation and is left here
    // in case a curious reader needs it.
    #[allow(dead_code)]
    fn size(&self) -> usize {
        if self.wraps > 0 {
            // the index wrapped at least once, so the ring buffer is definitely full.
            self.data.len()
        } else {
            // index hasn't wrapped yet, so it counts the number of samples recorded in this buffer.
            self.idx
        }
    }
}

/// A sample of the state in [`Observations`].
#[derive(Debug, PartialEq, Eq)]
pub struct Sample<T: Ord + Zero + Copy> {
    /// Number of observations dropped due to lock contention
    pub dropped: usize,
    /// Number of times the observation window wrapped around
    pub wraps: usize,
    /// 25th percentile observation
    pub p25: T,
    /// 50th percentile observation
    pub p50: T,
    /// 75th percentile observation
    pub p75: T,
    /// 90th percentile observation
    pub p90: T,
    /// 95th percentile observation
    pub p95: T,
    /// 99th percentile observation
    pub p99: T,
    /// 99.9th percentile observation
    pub p99p9: T,
    /// Maximum observation
    pub max: T,
    /// Number of observations
    pub count: usize,
}

/// Collect observations, which are sampled as a [`Sample`].
pub struct Observations<T: Ord + Zero + Copy> {
    observations: Mutex<ObservationSet<T>>,
    drops: AtomicUsize,
    name: &'static str,
}

impl<T: Ord + Zero + Copy> Observations<T> {
    /// Constructor. The `name` parameter has no semantic meaning, and is only
    /// exposed by [`Observations::name()`].
    pub fn new(name: &'static str) -> Self {
        Self {
            observations: Mutex::new(ObservationSet::new()),
            drops: AtomicUsize::new(0),
            name,
        }
    }

    /// Name associated with the observations, as provided in constructor.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Take a sample of the observations. Calculates a [`Sample`] corresponding to the current
    /// state, and then clears that state.
    pub fn sample(&self) -> Sample<T> {
        let mut observations = self.observations.lock();
        let wraps = observations.wraps();
        let sorted = observations.sorted_data();

        fn percentile<T: Ord + Zero + Copy>(sorted_ts: &[T], p: f64) -> T {
            if sorted_ts.len() == 0 {
                T::zero()
            } else {
                let percentile_idx = ((sorted_ts.len() as f64 * p) / 100.0) as usize;
                sorted_ts[percentile_idx]
            }
        }
        let p25 = percentile(&sorted, 25.0);
        let p50 = percentile(&sorted, 50.0);
        let p75 = percentile(&sorted, 75.0);
        let p90 = percentile(&sorted, 90.0);
        let p95 = percentile(&sorted, 95.0);
        let p99 = percentile(&sorted, 99.0);
        let p99p9 = percentile(&sorted, 99.9);
        let max = sorted.last().map(|x| *x).unwrap_or_else(|| T::zero());
        let count = sorted.len();
        observations.clear();
        std::mem::drop(observations);

        // now that we've unblocked writing new observations, no more will be dropped, and we can
        // reset the drop count to 0
        let dropped = self.drops.swap(0, Ordering::SeqCst);
        Sample {
            dropped,
            wraps,
            p25,
            p50,
            p75,
            p90,
            p95,
            p99,
            p99p9,
            max,
            count,
        }
    }

    /// Attempt to record this `T` as part of the collection of observations. "Attempt", because if
    /// a reader is currently using this `ObservationSet`, the observation is dropped. This
    /// prevents recording from being a blocking operation
    pub fn record(&self, observation: T) {
        if let Some(mut observations) = self.observations.try_lock() {
            observations.add(observation);
        } else {
            // something else is using the data right now, just drop the observation
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
}

crate::label_enum! {
    /// Labels corresponding to the fields in [`Sample`]
    pub enum TimingBucket {
        /// 25th percentile observation
        P25,
        /// 50th percentile observation
        P50,
        /// 75th percentile observation
        P75,
        /// 90th percentile observation
        P90,
        /// 95th percentile observation
        P95,
        /// 99th percentile observation
        P99,
        /// 99.9th percentile observation
        P99P9,
        /// Maximum observation
        Max,
        /// Number of observations
        Count,
    }
}

impl Labels for TimingBucket {
    fn label_names() -> Vec<&'static str> {
        vec!["bucket"]
    }
    fn possible_label_values() -> Vec<LabelValues<'static>> {
        Self::all_variants()
            .into_iter()
            .map(|b| vec![b.as_str()])
            .collect()
    }
    fn label_values(&self) -> LabelValues {
        vec![self.as_str()]
    }
}

impl<T: Ord + Zero + Copy + Into<i64>> Sample<T> {
    /// Returns each member of the struct along with its [`TimingBucket`]
    /// label.  Each percentile is given as an i64.
    pub fn as_bucket_pairs(&self) -> Vec<(TimingBucket, i64)> {
        vec![
            (TimingBucket::P25, self.p25.into()),
            (TimingBucket::P50, self.p50.into()),
            (TimingBucket::P75, self.p75.into()),
            (TimingBucket::P90, self.p90.into()),
            (TimingBucket::P95, self.p95.into()),
            (TimingBucket::P99, self.p99.into()),
            (TimingBucket::P99P9, self.p99p9.into()),
            (TimingBucket::Max, self.max.into()),
            (TimingBucket::Count, self.count as i64),
        ]
    }

    /// Returns the number of observations dropped due to the observation lock
    /// being held.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// Returns the number of times the observation count exceeded the available
    /// window size.
    pub fn wraps(&self) -> usize {
        self.wraps
    }
}

#[cfg(test)]
mod tests {
    use super::{Observations, Sample, WINDOW_SIZE};

    #[test]
    fn test_wraps_are_reported() {
        let observations = Observations::new("test");

        for i in 0..WINDOW_SIZE {
            observations.record(i);
        }

        observations.record(500);
        observations.record(501);
        observations.record(502);
        observations.record(503);

        let sample = observations.sample();

        // `dropped` counts the number of samples that didn't make it to the underlying ring
        // buffer, but excessive samples are not dropped! overflows start writing over the start of
        // the buffer, and increment `wraps`.
        assert_eq!(sample.dropped, 0);
        assert_eq!(sample.wraps, 1);

        // sample again to confirm that defaults are zero and that wraps have not occurred since
        // the last sample.
        let sample = observations.sample();

        assert_eq!(
            sample,
            Sample {
                dropped: 0,
                wraps: 0,
                p25: 0,
                p50: 0,
                p75: 0,
                p90: 0,
                p95: 0,
                p99: 0,
                p99p9: 0,
                max: 0,
                count: 0,
            }
        );
    }

    #[test]
    fn test_percentiles_are_reported() {
        #[rustfmt::skip]
        let data = [
                 1,  2,  3,  4,  5,  6,  7,  8,  9,
            10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29,
            30, 31, 32, 33, 34, 35, 36, 37, 38, 39,
            40, 41, 42, 43, 44, 45, 46, 47, 48, 49,
            50, 51, 52, 53, 54, 55, 56, 57, 58, 59,
            60, 61, 62, 63, 64, 65, 66, 67, 68, 69,
            70, 71, 72, 73, 74, 75, 76, 77, 78, 79,
            80, 81, 82, 83, 84, 85, 86, 87, 88, 89,
            90, 91, 92, 93, 94, 95, 96, 97, 98, 99,
        ];

        let observations = Observations::new("test");

        for datum in data.iter().cloned() {
            observations.record(datum);
        }

        let sample = observations.sample();

        assert_eq!(
            sample,
            Sample {
                dropped: 0,
                wraps: 0,
                p25: 25,
                p50: 50,
                p75: 75,
                p90: 90,
                p95: 95,
                p99: 99,
                p99p9: 99,
                max: 99,
                count: 99,
            }
        );
    }

    #[test]
    fn test_small_sampleset() {
        let observations = Observations::new("test");

        observations.record(500);
        observations.record(501);
        observations.record(502);
        observations.record(503);
        observations.record(504);

        let sample = observations.sample();

        assert_eq!(
            sample,
            Sample {
                dropped: 0,
                wraps: 0,
                p25: 501,
                p50: 502,
                p75: 503,
                p90: 504,
                p95: 504,
                p99: 504,
                p99p9: 504,
                max: 504,
                count: 5,
            }
        );
    }

    #[test]
    fn test_overflow_wraps_writes() {
        let observations = Observations::new("test");

        for _ in 0..WINDOW_SIZE {
            observations.record(1);
        }

        // at this point, we've wrapped the window, and start overwriting `1` samples.
        for _ in 0..(WINDOW_SIZE / 2) {
            observations.record(2);
        }

        for _ in 0..(WINDOW_SIZE / 10) {
            observations.record(3);
        }

        let sample = observations.sample();

        assert_eq!(
            sample,
            Sample {
                dropped: 0,
                wraps: 1,
                p25: 2,
                p50: 2,
                p75: 2,
                p90: 3,
                p95: 3,
                p99: 3,
                p99p9: 3,
                max: 3,
                count: WINDOW_SIZE / 2 + WINDOW_SIZE / 10,
            }
        );
    }
}
