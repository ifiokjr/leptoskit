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
use std::hash::Hash;
use std::hash::Hasher;

use serde::Deserialize;
use serde::Serialize;

/// Identifies one query or mutation scope in the cache.
///
/// Anonymous scopes derive their id from the fetcher's type, so a scope built
/// in a plain function is the same scope on every call. Type ids are only
/// stable within one compiled binary, so a scope whose data crosses a process
/// boundary — dehydrated on the server, persisted to storage — must be named
/// with [`QueryScope::with_name`](crate::QueryScope::with_name): named ids are
/// the same on every target and every Rust release.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ScopeId(u64);

impl ScopeId {
	/// The id for a fetcher of type `F` over these key, data, and error types.
	///
	/// Stable only within one compiled binary.
	#[must_use]
	pub fn of<F: 'static, K: 'static, T: 'static, E: 'static>() -> Self {
		let mut hasher = StableHasher::new();
		TypeId::of::<F>().hash(&mut hasher);
		TypeId::of::<K>().hash(&mut hasher);
		TypeId::of::<T>().hash(&mut hasher);
		TypeId::of::<E>().hash(&mut hasher);
		Self(hasher.finish())
	}

	/// An id derived from a name, the same on every target and toolchain.
	#[must_use]
	pub fn named(name: &str) -> Self {
		let mut hasher = StableHasher::new();
		hasher.write(b"leptoskit_query::named_scope\xff");
		hasher.write(name.as_bytes());
		Self(hasher.finish())
	}

	/// The underlying hash.
	#[must_use]
	pub const fn raw(self) -> u64 {
		self.0
	}
}

/// Hashes a key for cache lookup and for matching dehydrated data.
///
/// The hash is the same in every process that hashes the same key value: the
/// server and the browser, a 64-bit host and wasm32 (`usize`, `isize`, and
/// collection lengths are hashed as 64-bit values), and builds made with
/// different Rust releases. It follows the key's [`Hash`] implementation, so
/// a key type must hash the same way wherever it is compiled, which std types
/// and `#[derive(Hash)]` do.
///
/// Within one cache, keys are also compared for equality, so two keys whose
/// hashes collide are still separate queries.
#[must_use]
pub fn hash_key<K: Hash + ?Sized>(key: &K) -> u64 {
	let mut hasher = StableHasher::new();
	key.hash(&mut hasher);
	hasher.finish()
}

/// 64-bit FNV-1a over a fixed little-endian encoding of every integer.
///
/// Unlike `DefaultHasher`, whose algorithm is unspecified and whose `usize`
/// writes follow the target's pointer width, the output depends only on the
/// sequence of values hashed.
struct StableHasher(u64);

impl StableHasher {
	const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
	const PRIME: u64 = 0x0000_0100_0000_01b3;

	const fn new() -> Self {
		Self(Self::OFFSET_BASIS)
	}
}

impl Hasher for StableHasher {
	fn finish(&self) -> u64 {
		self.0
	}

	fn write(&mut self, bytes: &[u8]) {
		for byte in bytes {
			self.0 ^= u64::from(*byte);
			self.0 = self.0.wrapping_mul(Self::PRIME);
		}
	}

	fn write_u8(&mut self, value: u8) {
		self.write(&[value]);
	}

	fn write_u16(&mut self, value: u16) {
		self.write(&value.to_le_bytes());
	}

	fn write_u32(&mut self, value: u32) {
		self.write(&value.to_le_bytes());
	}

	fn write_u64(&mut self, value: u64) {
		self.write(&value.to_le_bytes());
	}

	fn write_u128(&mut self, value: u128) {
		self.write(&value.to_le_bytes());
	}

	fn write_usize(&mut self, value: usize) {
		// `usize` is at most 64 bits on every supported target.
		self.write_u64(value as u64);
	}

	fn write_i8(&mut self, value: i8) {
		self.write(&value.to_le_bytes());
	}

	fn write_i16(&mut self, value: i16) {
		self.write(&value.to_le_bytes());
	}

	fn write_i32(&mut self, value: i32) {
		self.write(&value.to_le_bytes());
	}

	fn write_i64(&mut self, value: i64) {
		self.write(&value.to_le_bytes());
	}

	fn write_i128(&mut self, value: i128) {
		self.write(&value.to_le_bytes());
	}

	fn write_isize(&mut self, value: isize) {
		// `isize` is at most 64 bits on every supported target.
		self.write_i64(value as i64);
	}
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
