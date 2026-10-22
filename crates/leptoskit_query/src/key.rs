//! Scope identity, key hashing, and hierarchical tags.
//!
//! TanStack Query addresses a query with an array key like
//! `['todos', { page: 2 }]`, and `['todos']` partially matches every todo
//! query. Here the first part — *which fetcher* — is the typed
//! [`QueryScope`](crate::QueryScope), identified by a [`ScopeId`], and the
//! rest is the typed key `K`. Cross-cutting invalidation ("everything about
//! user 7") uses [`Tag`]s: hierarchical paths a scope attaches to its keys,
//! matched by prefix exactly like TanStack's partial keys.

use std::any::TypeId;
use std::fmt;
use std::hash::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;

use serde::Deserialize;
use serde::Serialize;

/// Identifies one query or mutation scope in the cache.
///
/// Anonymous scopes derive their id from the fetcher's type, so a scope built
/// in a plain function is the same scope on every call. Name a scope with
/// [`QueryScope::with_name`](crate::QueryScope::with_name) when its identity
/// must survive across builds — persisted caches, for instance — since type
/// ids are only stable within one compiled binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ScopeId(u64);

impl ScopeId {
	/// The id for a fetcher of type `F` over these key, data, and error types.
	#[must_use]
	pub fn of<F: 'static, K: 'static, T: 'static, E: 'static>() -> Self {
		let mut hasher = DefaultHasher::new();
		TypeId::of::<F>().hash(&mut hasher);
		TypeId::of::<K>().hash(&mut hasher);
		TypeId::of::<T>().hash(&mut hasher);
		TypeId::of::<E>().hash(&mut hasher);
		Self(hasher.finish())
	}

	/// A stable id derived from a name.
	#[must_use]
	pub fn named(name: &str) -> Self {
		let mut hasher = DefaultHasher::new();
		"leptoskit_query::named_scope".hash(&mut hasher);
		name.hash(&mut hasher);
		Self(hasher.finish())
	}

	/// The underlying hash.
	#[must_use]
	pub const fn raw(self) -> u64 {
		self.0
	}
}

/// Hashes a key for cache lookup.
///
/// `DefaultHasher::new()` uses fixed keys, so the hash is deterministic for
/// one binary, which persisted caches rely on.
#[must_use]
pub fn hash_key<K: Hash + ?Sized>(key: &K) -> u64 {
	let mut hasher = DefaultHasher::new();
	key.hash(&mut hasher);
	hasher.finish()
}

/// A hierarchical label for cross-scope invalidation.
///
/// Tags match by prefix: `Tag::new(["users"])` matches a query tagged
/// `["users", "7"]` but not the other way round.
///
/// ```
/// use leptoskit_query::Tag;
///
/// let user = Tag::new(["users", "7"]);
/// assert!(user.starts_with(&Tag::new(["users"])));
/// assert!(!Tag::new(["users"]).starts_with(&user));
/// assert_eq!(Tag::from("users").child(7), user);
/// ```
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Tag(Vec<String>);

impl Tag {
	/// A tag from its path segments.
	pub fn new(segments: impl IntoIterator<Item = impl Into<String>>) -> Self {
		Self(segments.into_iter().map(Into::into).collect())
	}

	/// The path segments.
	#[must_use]
	pub fn segments(&self) -> &[String] {
		&self.0
	}

	/// Whether this tag sits at or below `prefix`.
	#[must_use]
	pub fn starts_with(&self, prefix: &Tag) -> bool {
		self.0.starts_with(&prefix.0)
	}

	/// This tag with one more segment appended.
	#[must_use]
	pub fn child(mut self, segment: impl fmt::Display) -> Self {
		self.0.push(segment.to_string());
		self
	}
}

impl fmt::Debug for Tag {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_list().entries(&self.0).finish()
	}
}

impl fmt::Display for Tag {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(&self.0.join("/"))
	}
}

impl From<&str> for Tag {
	fn from(segment: &str) -> Self {
		Self(vec![segment.to_owned()])
	}
}

impl From<String> for Tag {
	fn from(segment: String) -> Self {
		Self(vec![segment])
	}
}

impl From<Vec<String>> for Tag {
	fn from(segments: Vec<String>) -> Self {
		Self(segments)
	}
}

impl<const N: usize> From<[&str; N]> for Tag {
	fn from(segments: [&str; N]) -> Self {
		Self::new(segments)
	}
}

/// Builds a [`Tag`] from `Display` segments.
///
/// ```
/// use leptoskit_query::Tag;
/// use leptoskit_query::tag;
///
/// let user_id = 7;
/// assert_eq!(tag!["users", user_id], Tag::new(["users", "7"]));
/// ```
#[macro_export]
macro_rules! tag {
	($($segment:expr),+ $(,)?) => {
		$crate::Tag::new([$(::std::string::ToString::to_string(&$segment)),+])
	};
}
