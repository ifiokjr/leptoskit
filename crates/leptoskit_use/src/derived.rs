use std::cmp::Ordering;
use std::fmt::Display;

use reactive_graph::prelude::*;
use reactive_graph::wrappers::read::Signal;

/// Convert a reactive value to its display representation.
pub fn use_to_string<T>(value: impl With<Value = T> + Send + Sync + 'static) -> Signal<String>
where
	T: Display + Send + Sync + 'static,
{
	Signal::derive(move || value.with(ToString::to_string))
}

/// Negate a reactive boolean.
pub fn use_not(value: impl Into<Signal<bool>>) -> Signal<bool> {
	let value = value.into();
	Signal::derive(move || !value.get())
}

/// Logical AND of two reactive booleans.
pub fn use_and(a: impl Into<Signal<bool>>, b: impl Into<Signal<bool>>) -> Signal<bool> {
	let (a, b) = (a.into(), b.into());
	Signal::derive(move || a.get() && b.get())
}

/// Logical OR of two reactive booleans.
pub fn use_or(a: impl Into<Signal<bool>>, b: impl Into<Signal<bool>>) -> Signal<bool> {
	let (a, b) = (a.into(), b.into());
	Signal::derive(move || a.get() || b.get())
}

/// Whether a reactive optional value is present.
pub fn is_some<T: Send + Sync + 'static>(
	value: impl With<Value = Option<T>> + Send + Sync + 'static,
) -> Signal<bool> {
	Signal::derive(move || value.with(Option::is_some))
}

/// Whether a reactive optional value is absent.
pub fn is_none<T: Send + Sync + 'static>(
	value: impl With<Value = Option<T>> + Send + Sync + 'static,
) -> Signal<bool> {
	Signal::derive(move || value.with(Option::is_none))
}

/// Whether a reactive result succeeded.
pub fn is_ok<T: Send + Sync + 'static, E: Send + Sync + 'static>(
	value: impl With<Value = Result<T, E>> + Send + Sync + 'static,
) -> Signal<bool> {
	Signal::derive(move || value.with(Result::is_ok))
}

/// Whether a reactive result failed.
pub fn is_err<T: Send + Sync + 'static, E: Send + Sync + 'static>(
	value: impl With<Value = Result<T, E>> + Send + Sync + 'static,
) -> Signal<bool> {
	Signal::derive(move || value.with(Result::is_err))
}

/// Sort a copy of a reactive vector without changing its source.
pub fn use_sorted<T>(value: impl Into<Signal<Vec<T>>>) -> Signal<Vec<T>>
where
	T: Clone + Ord + Send + Sync + 'static,
{
	use_sorted_by(value, Ord::cmp)
}

/// Sort a copy with a custom comparator. Equal items retain their order.
pub fn use_sorted_by<T>(
	value: impl Into<Signal<Vec<T>>>,
	compare: impl Fn(&T, &T) -> Ordering + Send + Sync + 'static,
) -> Signal<Vec<T>>
where
	T: Clone + Send + Sync + 'static,
{
	let value = value.into();
	Signal::derive(move || {
		let mut sorted = value.get();
		sorted.sort_by(&compare);
		sorted
	})
}

/// Sort a copy by a key. Equal keys retain their order.
pub fn use_sorted_by_key<T, K>(
	value: impl Into<Signal<Vec<T>>>,
	key: impl Fn(&T) -> K + Send + Sync + 'static,
) -> Signal<Vec<T>>
where
	T: Clone + Send + Sync + 'static,
	K: Ord,
{
	let value = value.into();
	Signal::derive(move || {
		let mut sorted = value.get();
		sorted.sort_by_key(&key);
		sorted
	})
}

/// Reactive absolute value, with Rust's `f64` NaN and infinity semantics.
pub fn use_abs(value: impl Into<Signal<f64>>) -> Signal<f64> {
	let value = value.into();
	Signal::derive(move || value.get().abs())
}

/// Reactive ceiling.
pub fn use_ceil(value: impl Into<Signal<f64>>) -> Signal<f64> {
	let value = value.into();
	Signal::derive(move || value.get().ceil())
}

/// Reactive floor.
pub fn use_floor(value: impl Into<Signal<f64>>) -> Signal<f64> {
	let value = value.into();
	Signal::derive(move || value.get().floor())
}

/// Reactive rounding to the nearest integer, ties away from zero.
pub fn use_round(value: impl Into<Signal<f64>>) -> Signal<f64> {
	let value = value.into();
	Signal::derive(move || value.get().round())
}

/// Minimum of a reactive collection; empty collections return `None`.
pub fn use_min<T>(value: impl Into<Signal<Vec<T>>>) -> Signal<Option<T>>
where
	T: Clone + Ord + Send + Sync + 'static,
{
	let value = value.into();
	Signal::derive(move || value.with(|items| items.iter().min().cloned()))
}

/// Maximum of a reactive collection; empty collections return `None`.
pub fn use_max<T>(value: impl Into<Signal<Vec<T>>>) -> Signal<Option<T>>
where
	T: Clone + Ord + Send + Sync + 'static,
{
	let value = value.into();
	Signal::derive(move || value.with(|items| items.iter().max().cloned()))
}
