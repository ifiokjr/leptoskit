use std::sync::Arc;

use reactive_graph::prelude::*;
use reactive_graph::signal::RwSignal;
use reactive_graph::signal::WriteSignal;
use reactive_graph::wrappers::read::Signal;

use crate::Action;

/// An initial value or an existing writable signal.
#[derive(Debug, Clone, Copy)]
pub enum MaybeRwSignal<T: Send + Sync + 'static> {
	/// Allocate a new owner-scoped signal.
	Value(T),
	/// Reuse this signal, including its current value.
	Signal(RwSignal<T>),
}

impl<T: Send + Sync + 'static> From<T> for MaybeRwSignal<T> {
	fn from(value: T) -> Self {
		Self::Value(value)
	}
}

impl<T: Send + Sync + 'static> From<RwSignal<T>> for MaybeRwSignal<T> {
	fn from(value: RwSignal<T>) -> Self {
		Self::Signal(value)
	}
}

impl<T: Send + Sync + 'static> MaybeRwSignal<T> {
	/// Allocate or reuse the writable signal.
	pub fn into_signal(self) -> RwSignal<T> {
		match self {
			Self::Value(value) => RwSignal::new(value),
			Self::Signal(signal) => signal,
		}
	}
}

/// Boolean state and its controls.
#[derive(Clone)]
pub struct UseToggleReturn {
	/// Current value.
	pub value: Signal<bool>,
	/// Set the value directly.
	pub set_value: WriteSignal<bool>,
	/// Invert the current value.
	pub toggle: Action,
}

/// A boolean switch, optionally sharing an existing signal.
pub fn use_toggle(initial: impl Into<MaybeRwSignal<bool>>) -> UseToggleReturn {
	let value = initial.into().into_signal();
	UseToggleReturn {
		value: value.into(),
		set_value: value.write_only(),
		toggle: Arc::new(move || value.update(|value| *value = !*value)),
	}
}

/// A cyclic selection. An empty list has no selected index or value.
#[derive(Clone)]
pub struct UseCycleListReturn<T: Send + Sync + 'static> {
	/// Current item, or `None` when the list is empty.
	pub state: Signal<Option<T>>,
	/// Current index, normalized against the latest list length.
	pub index: Signal<Option<usize>>,
	/// Select an index, wrapping at the current list length.
	pub set_index: Arc<dyn Fn(usize) + Send + Sync>,
	/// Advance by one item.
	pub next: Action,
	/// Move back by one item.
	pub prev: Action,
}

/// Cycle through a reactive list. Shrinking and empty lists remain valid.
///
/// Selection follows the index, not item identity. Empty lists expose no
/// selection; the stored index is reused when populated again.
pub fn use_cycle_list<T>(list: impl Into<Signal<Vec<T>>>) -> UseCycleListReturn<T>
where
	T: Clone + Send + Sync + 'static,
{
	let list = list.into();
	let position = RwSignal::new(0_usize);
	let index = Signal::derive(move || {
		list.with(|items| (!items.is_empty()).then(|| position.get() % items.len()))
	});
	let state = Signal::derive(move || list.with(|items| index.get().map(|i| items[i].clone())));
	UseCycleListReturn {
		state,
		index,
		set_index: Arc::new(move |i| position.set(i)),
		next: Arc::new(move || {
			let i = index.get_untracked().unwrap_or(0);
			position.set(list.with_untracked(|items| {
				if items.is_empty() || i == items.len() - 1 {
					0
				} else {
					i + 1
				}
			}));
		}),
		prev: Arc::new(move || {
			let i = index.get_untracked().unwrap_or(0);
			position.set(list.with_untracked(|items| {
				if i == 0 {
					items.len().saturating_sub(1)
				} else {
					i - 1
				}
			}));
		}),
	}
}

/// Present platform capability detection as a signal.
///
/// Wrap a static `available()` probe from a host or plugin so shared screens
/// can branch reactively on it. The value never changes unless the underlying
/// closure reads other signals.
pub fn use_supported(available: impl Fn() -> bool + Send + Sync + 'static) -> Signal<bool> {
	Signal::derive(available)
}
