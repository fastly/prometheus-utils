use crate::{IntCounterWithLabels, Labels};
use prometheus::core::{Atomic, AtomicF64, AtomicI64, GenericCounter, GenericGauge, Number};

/// An RAII-style guard for an [`AtomicI64`] gauge.
///
/// Created by the methods on the [`GuardedGauge`] extension trait.
pub type IntGaugeGuard = GenericGaugeGuard<AtomicI64>;

/// An RAII-style guard for an [`AtomicF64`] gauge.
///
/// Created by the methods on the [`GuardedGauge`] extension trait.
pub type GaugeGuard = GenericGaugeGuard<AtomicF64>;

/// An RAII-style guard for situations where we want to increment a gauge and then ensure that there
/// is always a corresponding decrement.
///
/// Created by the methods on the [`GuardedGauge`] extension trait.
pub struct GenericGaugeGuard<P: Atomic + 'static> {
    value: P::T,
    gauge: &'static GenericGauge<P>,
}

/// When a gauge guard is dropped, it will perform the corresponding decrement.
impl<P: Atomic + 'static> Drop for GenericGaugeGuard<P> {
    fn drop(&mut self) {
        self.gauge.sub(self.value);
    }
}

/// An extension trait for [`prometheus::GenericGauge`] to provide methods for temporarily
/// modifying a gauge.
pub trait GuardedGauge<P: Atomic + 'static> {
    /// Increase the gauge by 1 while the guard exists.
    #[must_use]
    fn guarded_inc(&'static self) -> GenericGaugeGuard<P>;

    /// Increase the gauge by the given increment while the guard exists.
    #[must_use]
    fn guarded_add(&'static self, v: P::T) -> GenericGaugeGuard<P>;
}

impl<P: Atomic + 'static> GuardedGauge<P> for GenericGauge<P> {
    fn guarded_inc(&'static self) -> GenericGaugeGuard<P> {
        self.inc();
        GenericGaugeGuard {
            value: <P::T as Number>::from_i64(1),
            gauge: self,
        }
    }

    fn guarded_add(&'static self, v: P::T) -> GenericGaugeGuard<P> {
        self.add(v);
        GenericGaugeGuard {
            value: v,
            gauge: self,
        }
    }
}

/// A guard that will automatically increment a labeled metric when dropped.
///
/// Created by calling [`IntCounterWithLabels::deferred_inc`].
pub struct DeferredIncWithLabels<'a, L: Labels> {
    metric: &'a IntCounterWithLabels<L>,
    labels: &'a L,
}

/// When dropped, a [`DeferredIncWithLabels`] guard will increment its counter.
impl<'a, L: Labels> Drop for DeferredIncWithLabels<'a, L> {
    fn drop(&mut self) {
        self.metric.inc(&self.labels)
    }
}

impl<'a, L: Labels> DeferredIncWithLabels<'a, L> {
    /// Create a new deferred increment guard.
    //
    // This is not exposed in the public interface, these should only be acquired through
    // `deferred_inc`.
    pub(crate) fn new(metric: &'a IntCounterWithLabels<L>, labels: &'a L) -> Self {
        Self { metric, labels }
    }

    /// Update the labels to use when incrementing the metric.
    pub fn with_labels<'new_labels>(
        self,
        new_labels: &'new_labels L,
    ) -> DeferredIncWithLabels<'new_labels, L>
    where
        'a: 'new_labels,
    {
        DeferredIncWithLabels {
            metric: self.metric,
            labels: new_labels,
        }
    }

    /// Eagerly perform the increment consume the guard.
    pub fn inc(self) {
        drop(self)
    }
}

/// A guard that will automatically increment a [`GenericCounter`] when dropped.
///
/// Created by the methods on the [`DeferredCounter`] extension trait.
pub struct DeferredAdd<P: Atomic + 'static> {
    value: P::T,
    metric: &'static GenericCounter<P>,
}

/// When dropped, a [`DeferredAdd`] guard will increment its counter.
impl<P: Atomic + 'static> Drop for DeferredAdd<P> {
    fn drop(&mut self) {
        self.metric.inc_by(self.value);
    }
}

/// An extension trait for [`prometheus::GenericCounter`] to provide methods for incrementing a
/// counter after an RAII-style guard has been dropped.
pub trait DeferredCounter<P: Atomic + 'static> {
    /// Increase the counter by `1` when the guard is dropped.
    #[must_use]
    fn deferred_inc(&'static self) -> DeferredAdd<P> {
        self.deferred_add(<P::T as Number>::from_i64(1))
    }

    /// Increase the counter by `v` when the guard is dropped.
    #[must_use]
    fn deferred_add(&'static self, v: P::T) -> DeferredAdd<P>;
}

impl<P: Atomic + 'static> DeferredCounter<P> for GenericCounter<P> {
    fn deferred_add(&'static self, v: P::T) -> DeferredAdd<P> {
        DeferredAdd {
            value: v,
            metric: self,
        }
    }
}
