use crate::guards::DeferredInc;
use prometheus::{register_int_counter_vec, register_int_gauge_vec, IntCounterVec, IntGaugeVec};
use std::marker::PhantomData;

/// A sequence of values for Prometheus labels
pub type LabelValues<'a> = Vec<&'a str>;

/// The `Labels` trait applies to values intended to generate Prometheus labels for a metric.
///
/// A metric in Prometheus can include any number of labels. Each label has a fixed name,
/// and when events are emitted for the metric, those events must include values for each
/// of the labels. Using labels makes it possible to easily see a metric in aggregate (i.e.,
/// to see totals regardless of label values), or to query specific kinds of events by
/// filtering label values.
///
/// Thus, for example, rather than having a separate metric for each kind of error that
/// arises, we can produce one metric with an "err" label, whose value will reflect which
/// error has occurred. That simplifies the top-level list of metrics, and makes it easier
/// to build queries to aggregate specific kinds of error events.
///
/// This trait adds some extra guard rails on top of the prometheus-rs crate, so that when
/// we emit a labeled metric we can use a custom type to represent the labels, rather than
/// working directly with slices of string slices. When defining a labeled metric, you should
/// also define a new type representing its labels that implements the `Labels` trait.
/// Then, when emitting events for the metric, labels are passed in using this custom type,
/// which rules out several kinds of bugs (like missing or incorrectly ordered label values).
///
/// You can define labeled metrics using types like [`IntCounterWithLabels`], which are
/// parameterized by a type that implements `Labels`.
///
/// [`IntCounterWithLabels`]: struct.IntCounterWithLabels.html
pub trait Labels {
    /// The names of the labels that will be defined for the corresponding metric.
    fn label_names() -> Vec<&'static str>;

    /// Labels values to seed the metric with initially.
    ///
    /// Since Prometheus doesn't know the possible values a label will take on, when we set
    /// up a labeled metric by default no values will appear until events are emitted. But
    /// for discoverability, it's helpful to initialize a labeled metric with some possible
    /// label values (at count 0) even if no events for the metric have occurred.
    ///
    /// The label values provided by this function are used to pre-populate the metric at
    /// count 0. **The values do _not_ need to be exhaustive**; it's fine for events to emit
    /// label values that are not included here.
    fn possible_label_values() -> Vec<LabelValues<'static>>;

    /// The actual label values to provide when emitting an event to Prometheus.
    ///
    /// The sequence of values should correspond to the names provided in `label_names`,
    /// in order.
    fn label_values(&self) -> LabelValues;
}

/// A Prometheus integer counter metric, with labels described by the type `L`.
///
/// The type `L` must implement the [`Labels`] trait; see the documentation for that trait
/// for an overview of Prometheus metric labels.
///
/// [`Labels`]: trait.Labels.html
pub struct IntCounterWithLabels<L: Labels> {
    metric: IntCounterVec,
    _labels: PhantomData<L>,
}

impl<L: Labels> IntCounterWithLabels<L> {
    /// Construct and immediately register a new `IntCounterWithLabels` instance.
    pub fn register_new(name: &str, help: &str) -> IntCounterWithLabels<L> {
        let metric = register_int_counter_vec!(name, help, &L::label_names()).unwrap();

        for vals in L::possible_label_values() {
            metric.with_label_values(&vals).inc_by(0);
        }

        Self {
            metric,
            _labels: PhantomData,
        }
    }

    /// Increment the metric using the provided `labels` for the event.
    pub fn inc(&self, labels: &L) {
        self.metric.with_label_values(&labels.label_values()).inc();
    }

    /// Creates a guard value that will increment the metric using the provided `labels` once dropped.
    ///
    /// Prior to dropping, the labels can be altered using [`DeferredInc::with_labels`].
    #[must_use]
    pub fn deferred_inc<'a>(&'a self, labels: &'a L) -> DeferredInc<'a, L> {
        DeferredInc::new(self, labels)
    }
}

/// A Prometheus integer gauge metric, with labels described by the type `L`.
///
/// The type `L` must implement the [`Labels`] trait; see the documentation for that trait
/// for an overview of Prometheus metric labels.
///
/// [`Labels`]: trait.Labels.html
pub struct IntGaugeWithLabels<L: Labels> {
    metric: IntGaugeVec,
    _labels: PhantomData<L>,
}

impl<L: Labels> IntGaugeWithLabels<L> {
    /// Construct and immediately register a new `IntGaugeWithLabels` instance.
    pub fn register_new(name: &str, help: &str) -> IntGaugeWithLabels<L> {
        let metric = register_int_gauge_vec!(name, help, &L::label_names()).unwrap();

        // Note: for gauges, unlike counters, we don't need to -- and should not! -- prepopulate
        // the metric with the possible labels. Unlike counters, which are only updated when an
        // event occurs, gauges _always_ have a value. Moreover, we cannot make assumptions about
        // an initial gauge value (unlike an initial 0 value for counters).

        Self {
            metric,
            _labels: PhantomData,
        }
    }

    /// Set the value of the gauge with the provided `labels`.
    pub fn set(&self, labels: &L, value: i64) {
        self.metric
            .with_label_values(&labels.label_values())
            .set(value);
    }
}
