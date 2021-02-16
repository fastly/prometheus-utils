use super::GuardedGauge;
#[cfg(test)]
use lazy_static::lazy_static;
use pin_project::pin_project;
use prometheus::core::{Atomic, GenericCounter};
use std::{future, ops::Deref, pin::Pin, task};

/// An instrumented [`Future`][std-future].
///
/// `InstrumentedFuture` provides a transparent observability layer for futures.  An instrumented
/// future is not created directly. Rather, an instrumented future is created _from_ an existing
/// future using [`IntoInstrumentedFuture::into_instrumented_future`][into-fut].
///
/// Most importantly, the [`increment_until_resolved`][incr-until] method allows callers to
/// increment a [`GuardedGauge`][guarded-gauge], and then decrement the gauge once the future has
/// resolved.
///
/// [guarded-gauge]: trait.GuardedGauge.html
/// [incr-until]: struct.InstrumentedFuture.html#method.increment_until_resolved
/// [into-fut]: trait.IntoInstrumentedFuture.html#tymethod.into_instrumented_future
/// [std-future]: https://doc.rust-lang.org/std/future/trait.Future.html
#[pin_project]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct InstrumentedFuture<F: future::Future> {
    /// The inner [`Future`][std-future].
    ///
    /// ## Structural Pinning
    ///
    /// Pinning is structural for `fut`. Because we delegate to it's
    /// [`Future::poll`][fut-poll] implementation, the inner future *must* be pinned if the outer
    /// `InstrumentedFuture` is pinned.
    ///
    /// See [`std::pin`][projections] and [`pin-project`][pin-project] for more information on
    /// structural pinning.
    ///
    /// [fut-poll]: https://doc.rust-lang.org/stable/core/future/trait.Future.html#tymethod.poll
    /// [pin-project]: https://docs.rs/pin-project/latest/pin_project/
    /// [projections]: https://doc.rust-lang.org/stable/std/pin/index.html#projections-and-structural-pinning
    /// [std-future]: https://doc.rust-lang.org/std/future/trait.Future.html
    #[pin]
    inner: F,
    /// Closures to call before polling the inner `Future`. These may, but are not required to,
    /// return items to be `Drop`ped when the inner `Future` completes or is cancelled.
    ///
    /// In practice, this holds a list of Prometheus counters or gauges to increment when the inner
    /// `Future` starts, returning a list of guards to decrement once the inner `Future` completes.
    pre_polls: Vec<Box<dyn FnOnce() -> Option<Box<dyn Drop + Send>> + Send>>,
    /// RAII guards that will be dropped once the future has resolved.
    ///
    /// In practice, this is used to hold values like [`IntGaugeGuard`][int-guard] and
    /// [`GaugeGuard`][float-guard], so that Prometheus metrics are properly decremented once the
    /// underlying future has been polled to completion.
    ///
    /// See [`increment_until_resolved`][inc-until] for more information.
    ///
    /// [float-guard]: type.GaugeGuard.html
    /// [int-guard]: type.IntGaugeGuard.html
    /// [inc-until]: struct.InstrumentedFuture.html#method.increment_until_resolved
    resource_guards: Vec<Box<dyn Drop + Send>>,
}

/// Convert a future into an instrumented future.
///
/// See the [`InstrumentedFuture`][instr-fut] documentation for more information.
///
/// [instr-fut]: struct.InstrumentedFuture.html
pub trait IntoInstrumentedFuture {
    type Future: future::Future;
    fn into_instrumented_future(self) -> InstrumentedFuture<Self::Future>;
}

impl<F: future::Future> IntoInstrumentedFuture for F {
    type Future = Self;
    fn into_instrumented_future(self) -> InstrumentedFuture<Self> {
        InstrumentedFuture {
            inner: self,
            pre_polls: vec![],
            resource_guards: vec![],
        }
    }
}

impl<F: future::Future> InstrumentedFuture<F> {
    /// Queue `guard_fn` to execute when the future is polled, retaining the returned value until
    /// the future completes.
    pub fn with_guard<GuardFn: FnOnce() -> Option<Box<dyn Drop + Send>> + Send + 'static>(
        mut self,
        guard_fn: GuardFn,
    ) -> Self {
        self.pre_polls.push(Box::new(guard_fn));
        self
    }

    /// Increment a Prometheus counter immediately.
    pub fn with_count<P: Atomic + 'static>(mut self, counter: &'static GenericCounter<P>) -> Self {
        self.pre_polls.push(Box::new(move || {
            counter.inc();
            None
        }));
        self
    }

    /// Increment a Prometheus gauge until this future has resolved.
    ///
    /// When called, this method will immediately increment the given gauge using the
    /// [`GuardedGauge::gaurded_inc`][gaurded-inc] trait method. This gauge will then be
    /// decremented once this future's [`Future::poll`][fut-poll] implementation returns a
    /// [`Poll::Ready`][poll-ready] value.
    ///
    /// See the [`GenericGaugeGuard`][gauge-guard] documentation for more information about RAII
    /// guards for Prometheus metrics.
    ///
    /// [fut-poll]: https://doc.rust-lang.org/stable/core/future/trait.Future.html#tymethod.poll
    /// [gauge-guard]: struct.GenericGaugeGuard.html
    /// [gaurded-inc]: trait.GuardedGauge.html#tymethod.guarded_inc
    /// [poll-ready]: https://doc.rust-lang.org/std/task/enum.Poll.html#variant.Ready
    pub fn with_count_gauge<G, T, P>(mut self, gauge: &'static G) -> Self
    where
        G: Deref<Target = T> + Sync,
        T: GuardedGauge<P> + 'static,
        P: Atomic + 'static,
    {
        self.pre_polls.push(Box::new(move || {
            Some(Box::new(gauge.deref().guarded_inc()))
        }));
        self
    }
}

impl<F: future::Future> future::Future for InstrumentedFuture<F> {
    /// An instrumented future returns the same type as its inner future.
    type Output = <F as future::Future>::Output;
    /// Polls the inner future.
    ///
    /// If the inner future's [`Future::poll`][fut-poll] implementation returns a
    /// [`Poll::Ready`][poll-ready] value, Prometheus gauges will be decremented accordingly.
    fn poll(self: Pin<&mut Self>, cx: &mut task::Context) -> task::Poll<Self::Output> {
        use task::Poll::{Pending, Ready};
        let pin_projection = self.project();
        for pre_poll in pin_projection.pre_polls.drain(..) {
            if let Some(droppable) = pre_poll() {
                pin_projection.resource_guards.push(droppable);
            }
        }
        match pin_projection.inner.poll(cx) {
            // The inner future is still pending...
            p @ Pending => p,
            // If we are here, the inner future resolved! Before returning we should drop any
            // resource guards that may have been attached to this future.
            out @ Ready(_) => {
                pin_projection.resource_guards.clear();
                out
            }
        }
    }
}

#[test]
fn counters_increment_only_when_futures_run() {
    use prometheus::{opts, register_int_counter, register_int_gauge, IntCounter, IntGauge};
    use std::sync::{atomic::AtomicU8, atomic::Ordering, Arc, Mutex};
    lazy_static! {
        static ref WORK_COUNTER: IntCounter = register_int_counter!(opts!(
            "work_counter",
            "the number of times `work()` has been called"
        ))
        .unwrap();
        static ref WORK_GAUGE: IntGauge =
            register_int_gauge!(opts!("work_gauge", "the number `work()` currently running"))
                .unwrap();
        static ref CAN_MEASURE: AtomicU8 = AtomicU8::new(0);
    }

    let work_stoppage = Arc::new(Mutex::new(0));

    async fn work(stop_ref: Arc<Mutex<usize>>) {
        CAN_MEASURE.store(1, Ordering::SeqCst);
        *stop_ref.lock().unwrap() = 4;
    }

    let stop_ref = Arc::clone(&work_stoppage);

    let value_lock = work_stoppage.lock().unwrap();

    // create a future to do some work, but don't run it yet
    let f = work(stop_ref)
        .into_instrumented_future()
        .with_count(&WORK_COUNTER)
        .with_count_gauge(&WORK_GAUGE);

    assert_eq!(WORK_COUNTER.get(), 0);
    assert_eq!(WORK_GAUGE.get(), 0);

    let mut rt = tokio::runtime::Builder::new()
        .threaded_scheduler()
        .build()
        .expect("can build runtime");
    let handle = rt.spawn(f);

    while CAN_MEASURE.load(Ordering::SeqCst) == 0 {
        // wait for a future point where we know we can sample the counters
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // we have started `f`, and so we have started `work`, but we have not allowed `work` to
    // complete, so the gauge should still be 1.
    assert_eq!(WORK_COUNTER.get(), 1);
    assert_eq!(WORK_GAUGE.get(), 1);

    std::mem::drop(value_lock);

    rt.block_on(handle).expect("can block on f");

    // now `f` is complete, so the gauge should once again be 0.
    assert_eq!(WORK_COUNTER.get(), 1);
    assert_eq!(WORK_GAUGE.get(), 0);

    // and confirm the mutex has been work'd
    assert_eq!(*work_stoppage.lock().unwrap(), 4);
}
