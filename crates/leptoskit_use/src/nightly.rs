//! Nightly-only call syntax for leptoskit values.
//!
//! These impls are compiled when the `nightly` feature is enabled *and* the
//! compiler is a nightly build. On stable toolchains the feature is inert, so
//! `--all-features` builds keep working.

use reactive_graph::traits::Get;

use crate::state::MaybeRwSignal;

fn current<T: Clone + Send + Sync + 'static>(source: &MaybeRwSignal<T>) -> T {
	match source {
		MaybeRwSignal::Value(value) => value.clone(),
		MaybeRwSignal::Signal(signal) => signal.get(),
	}
}

impl<T> FnOnce<()> for MaybeRwSignal<T>
where
	T: Clone + Send + Sync + 'static,
{
	type Output = T;

	#[inline]
	extern "rust-call" fn call_once(self, _args: ()) -> Self::Output {
		current(&self)
	}
}

impl<T> FnMut<()> for MaybeRwSignal<T>
where
	T: Clone + Send + Sync + 'static,
{
	#[inline]
	extern "rust-call" fn call_mut(&mut self, _args: ()) -> Self::Output {
		current(self)
	}
}

impl<T> Fn<()> for MaybeRwSignal<T>
where
	T: Clone + Send + Sync + 'static,
{
	#[inline]
	extern "rust-call" fn call(&self, _args: ()) -> Self::Output {
		current(self)
	}
}
