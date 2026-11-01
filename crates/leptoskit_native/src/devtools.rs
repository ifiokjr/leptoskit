//! `DevTools` match keys: one name for "the same element" on every platform.
//!
//! A node's key is built from its parent's key, the source location that
//! created it, and its index among the siblings created at that location.
//! The web renderer and the native surface compute it from the same element
//! tree, so a node picked in the browser finds its twin in the simulator.
//! Keys are FNV-1a hashes, written as 16 hex digits: the inputs are fixed by
//! the protocol, never by a platform's hasher.
//!
//! A native host that learns each node's source from its guest, as text,
//! computes the same keys with [`Origin::Source`], [`ordinals`], and
//! [`child`].

use std::collections::HashMap;
use std::panic::Location;

use crate::NativeKind;

const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// The key of a tree's root element.
pub const ROOT: u64 = OFFSET;

fn mix(mut hash: u64, bytes: &[u8]) -> u64 {
	for byte in bytes {
		hash ^= u64::from(*byte);
		hash = hash.wrapping_mul(PRIME);
	}

	hash
}

/// What identifies a node among its siblings: where it was created, or its
/// kind when there is no location.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin<'a> {
	/// The file, line, and column that created the node.
	Source(&'a str, u32, u32),
	/// The node's kind, for a node without a source location.
	Kind(NativeKind),
}

impl Origin<'static> {
	pub(crate) fn of(source: Option<&'static Location<'static>>, kind: NativeKind) -> Self {
		match source {
			Some(location) => Self::Source(location.file(), location.line(), location.column()),
			None => Self::Kind(kind),
		}
	}
}

/// The key of a child: its parent's key, its origin, and its index among
/// the siblings that share that origin.
#[must_use]
pub fn child(parent: u64, origin: Origin<'_>, ordinal: usize) -> u64 {
	let mut hash = mix(parent, b"/");

	match origin {
		Origin::Source(file, line, column) => {
			hash = mix(hash, file.as_bytes());
			hash = mix(hash, format!(":{line}:{column}").as_bytes());
		}
		Origin::Kind(kind) => hash = mix(hash, format!("{kind:?}").as_bytes()),
	}

	mix(hash, format!("#{ordinal}").as_bytes())
}

/// Each sibling's index among the siblings that share its origin, in order.
pub fn ordinals<'a>(origins: impl IntoIterator<Item = Origin<'a>>) -> Vec<usize> {
	let mut seen: HashMap<Origin, usize> = HashMap::new();
	origins
		.into_iter()
		.map(|origin| {
			let count = seen.entry(origin).or_insert(0);
			let ordinal = *count;
			*count += 1;
			ordinal
		})
		.collect()
}

std::thread_local! {
	/// The `#[component]`s whose bodies are running, innermost last.
	static COMPONENTS: std::cell::RefCell<Vec<&'static str>> = const {
		std::cell::RefCell::new(Vec::new())
	};
}

/// The innermost `#[component]` whose body is running.
pub(crate) fn current_component() -> Option<&'static str> {
	COMPONENTS.with(|stack| stack.borrow().last().copied())
}

pub(crate) fn enter_component(name: &'static str) {
	COMPONENTS.with(|stack| stack.borrow_mut().push(name));
}

pub(crate) fn leave_component() {
	COMPONENTS.with(|stack| {
		stack.borrow_mut().pop();
	});
}

/// A key as the protocol writes it.
#[must_use]
pub fn format(key: u64) -> String {
	format!("{key:016x}")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[track_caller]
	fn here() -> &'static Location<'static> {
		Location::caller()
	}

	#[test]
	fn siblings_from_one_place_are_told_apart_by_order() {
		let card = Origin::of(Some(here()), NativeKind::View);
		let title = Origin::of(Some(here()), NativeKind::Text);
		assert_eq!(ordinals([card, title, card, card]), [0, 0, 1, 2]);
		assert_ne!(child(ROOT, card, 0), child(ROOT, card, 1));
	}

	#[test]
	fn keys_depend_on_the_parent() {
		let origin = Origin::Kind(NativeKind::Text);
		let first = child(ROOT, Origin::Kind(NativeKind::View), 0);
		let second = child(ROOT, Origin::Kind(NativeKind::View), 1);
		assert_ne!(child(first, origin, 0), child(second, origin, 0));
	}

	#[test]
	fn keys_are_stable_across_builds() {
		// The web and native builds must agree, so the hash is pinned.
		let origin = Origin::Source("src/routes/index.rs", 12, 5);
		assert_eq!(format(child(ROOT, origin, 0)), "2d8e66fcf6d1f390");
	}
}
