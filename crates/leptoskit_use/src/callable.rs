//! Call syntax for [`MaybeRwSignal`] on every toolchain: `initial()` reads the
//! plain value or the wrapped signal.
//!
//! Stable Rust cannot implement the `Fn` traits for a type, so
//! `MaybeRwSignal` instead derefs to a `dyn Fn() -> T`, the way leptos 0.9's
//! signals do. Producing that `&dyn Fn` from `&self` takes the "callable
//! types" technique: a closure that captures nothing but a value with
//! `Self`'s layout has `Self`'s layout too, so `&self` can be read as a
//! reference to that closure. See <https://github.com/dtolnay/case-studies/tree/master/callable-types>.
#![allow(unsafe_code)]

use std::mem::MaybeUninit;
use std::ops::Deref;

use reactive_graph::traits::Get;

use crate::state::MaybeRwSignal;

impl<T: Clone + Send + Sync + 'static> MaybeRwSignal<T> {
	/// The plain value, or the wrapped signal's current value, tracked when
	/// read inside a reactive context.
	pub fn get(&self) -> T {
		match self {
			Self::Value(value) => value.clone(),
			Self::Signal(signal) => signal.get(),
		}
	}
}

impl<T: Clone + Send + Sync + 'static> Deref for MaybeRwSignal<T> {
	type Target = dyn Fn() -> T + Send + Sync;

	fn deref(&self) -> &Self::Target {
		let unread = MaybeUninit::<Self>::uninit();
		let read = move || {
			// SAFETY: this closure is only ever called through the reference
			// returned below, whose bytes are the live `MaybeRwSignal` behind
			// `self`, so the captured value is initialized.
			unsafe { unread.assume_init_ref() }.get()
		};
		// The technique needs the closure laid out exactly as `Self`: one
		// captured field and nothing else. Check it rather than assume it.
		assert_eq!(size_of_val(&read), size_of::<Self>());
		assert_eq!(align_of_val(&read), align_of::<Self>());
		// SAFETY: the closure type has the size and alignment of `Self`, its
		// only field is a `MaybeUninit<Self>`, and the result borrows `self`,
		// so it never outlives the value it reads.
		unsafe { reinterpret(&read, self) }
	}
}

/// Reads `source` as a `C`, the closure type of `witness`, for as long as
/// `source` is borrowed. Taking the closure as a witness fixes `C` to its
/// concrete type before the result becomes a `dyn Fn`.
///
/// # Safety
///
/// `C` must have the size and alignment of `S`, and a valid `S` must be a
/// valid `C`.
unsafe fn reinterpret<'a, C, S, T>(
	_witness: &C,
	source: &'a S,
) -> &'a (dyn Fn() -> T + Send + Sync + 'static)
where
	C: Fn() -> T + Send + Sync + 'static,
{
	// SAFETY: guaranteed by the caller.
	unsafe { &*std::ptr::from_ref(source).cast::<C>() }
}
