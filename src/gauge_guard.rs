use prometheus::core::{Atomic, GenericGauge, Number};

pub type IntGaugeGuard = GenericGaugeGuard<prometheus::core::AtomicI64>;
pub type GaugeGuard = GenericGaugeGuard<prometheus::core::AtomicF64>;

/// An RAII-style guard for situations where we want to increment a gauge and then ensure that there
/// is always a corresponding decrement.
///
/// Created by the methods on the `GuardedGauge` extension trait.
pub struct GenericGaugeGuard<P: Atomic + 'static> {
    value: P::T,
    gauge: &'static GenericGauge<P>,
}

impl<P: Atomic + 'static> Drop for GenericGaugeGuard<P> {
    fn drop(&mut self) {
        self.gauge.sub(self.value);
    }
}

/// An extension trait for `prometheus::GenericGauge` to provide methods for temporarily modifying a
/// gauge.
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
