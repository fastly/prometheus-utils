//! Utilities for working with Prometheus metrics in Rust
//!
//! This crate builds on the Promtheus crate to provide API with additional safety guardrails:
//!
//! * Use [`InstrumentedFuture`] to easily instrument futures with metric updates.
//! * Use [`GuardedGauge`] to work with gauges using an RAII-style guard that decrements
//!   the gauge upon drop.
//! * Use [`IntCounterWithLabels`] and [`IntGaugeWithLabels`] to produce labeled Prometheus
//!   metrics with a type-safe API.

// When building the project in release mode:
//   (1): Promote warnings into errors.
//   (2): Warn about public items that are missing documentation.
//   (3): Deny broken documentation links.
//   (4): Deny invalid codeblock attributes in documentation.
//   (5): Promote warnings in examples into errors, except for unused variables.
#![cfg_attr(not(debug_assertions), deny(warnings))]
#![cfg_attr(not(debug_assertions), warn(missing_docs))]
#![cfg_attr(not(debug_assertions), deny(broken_intra_doc_links))]
#![cfg_attr(not(debug_assertions), deny(invalid_codeblock_attributes))]
#![cfg_attr(not(debug_assertions), doc(test(attr(deny(warnings)))))]
#![cfg_attr(not(debug_assertions), doc(test(attr(allow(dead_code)))))]
#![cfg_attr(not(debug_assertions), doc(test(attr(allow(unused_variables)))))]

mod guards;
mod instrumented_future;
mod labels;

pub use guards::{
    DeferredAdd, DeferredCounter, DeferredIncWithLabels, GaugeGuard, GenericGaugeGuard,
    GuardedGauge, IntGaugeGuard,
};
pub use instrumented_future::{InstrumentedFuture, IntoInstrumentedFuture};
pub use labels::{IntCounterWithLabels, IntGaugeWithLabels, LabelValues, Labels};

// See `label_enum!` below; the factoring into two macros is to accomodate parsing
// multiple kinds of visibility annotations.
#[macro_export]
#[doc(hidden)]
macro_rules! __label_enum_internal {
    ($(#[$attr:meta])* ($($vis:tt)*) enum $N:ident { $($(#[$var_attr:meta])* $V:ident),* }) => {
        $(#[$attr])* $($vis)* enum $N { $($(#[$var_attr])* $V),* }

            paste::paste! {
                impl $N {
                    /// The name of this enum variant, as a string slice.
                    pub fn as_str(&self) -> &'static str {
                        match self {
                            $($N::$V => stringify!([<$V:snake>])),*
                        }
                    }

                    /// A vector containing one instance of each of the enum's variants.
                    pub fn all_variants() -> Vec<Self> {
                        vec![$($N::$V),*]
                    }
                }
            }
    };
}

/// Declare an enum intended to be used as a Prometheus label.
///
/// This helper macro can only be used to define enums consisting of tags without values.
/// Each tag corresponds to a possible Prometheus label value. The macro then generates
/// two functions, `as_str` and `all_variants`, as described in the example below. Those
/// funtions are intended to assist in implementing the [`Labels`] trait for a label struct
/// that contains the enum, ensuring a consistent conversion to strings for label values,
/// and that all possible variants are included when implementing `possible_label_values`.
///
/// [`Labels`]: trait.Labels.html
///
/// # Example
///
/// When using the macro, define exactly one `enum` inside, as you would normally:
///
/// ```ignore
/// label_enum! {
///     pub(crate) enum MyErrorLabel {
///         IoError,
///         TimeoutError,
///         MemoryError,
///     }
/// }
/// ```
///
/// The macro will declare the enum exactly as provided. But in addition, it will generate
/// two functions:
///
/// ```ignore
/// impl MyErrorLabel {
///     /// The name of this enum variant, as a string slice.
///     pub fn as_str(&self) -> &'static str { ... }
///
///     /// A vector containing one instance of each of the enum's variants.
///     pub fn all_variants() -> Vec<Self> { ... }
/// }
/// ```
#[macro_export(local_inner_macros)]
macro_rules! label_enum {
    ($(#[$attr:meta])* enum $N:ident { $($(#[$var_attr:meta])* $V:ident),* $(,)* }) => {
        __label_enum_internal!($(#[$attr])* () enum $N { $($(#[$var_attr])* $V),* });
    };
    ($(#[$attr:meta])* pub enum $N:ident { $($(#[$var_attr:meta])* $V:ident),* $(,)* }) => {
        __label_enum_internal!($(#[$attr])* (pub) enum $N { $($(#[$var_attr])* $V),* });
    };
    ($(#[$attr:meta])* pub ($($vis:tt)+) enum $N:ident { $($(#[$var_attr:meta])* $V:ident),* $(,)* }) => {
        __label_enum_internal!($(#[$attr])* (pub ($($vis)+)) enum $N { $($(#[$var_attr])* $V),* });
    };
}
