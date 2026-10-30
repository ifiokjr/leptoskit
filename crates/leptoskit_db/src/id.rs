//! Typed primary keys.

use std::cmp::Ordering;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::marker::PhantomData;
use std::str::FromStr;

use uuid::Uuid;

/// A row's identity: a UUID that knows which table it belongs to.
///
/// `Id<User>` and `Id<Post>` can't be mixed up, and a column of type `Id<T>`
/// is a foreign key to `T`. New IDs are UUIDv7, ordered by creation time, so
/// indexes on them stay compact. They serialize as hyphenated strings.
pub struct Id<T: ?Sized> {
	uuid: Uuid,
	table: PhantomData<fn() -> T>,
}

impl<T: ?Sized> Id<T> {
	/// A new, time-ordered ID.
	#[cfg(feature = "server")]
	pub fn new() -> Self {
		Self::from_uuid(Uuid::now_v7())
	}

	/// The ID with this UUID. Explicit, so that an `Id<Post>` never becomes
	/// an `Id<User>` by accident.
	pub const fn from_uuid(uuid: Uuid) -> Self {
		Self {
			uuid,
			table: PhantomData,
		}
	}

	/// The underlying UUID.
	pub const fn uuid(&self) -> Uuid {
		self.uuid
	}

	/// Parses an ID from its hyphenated or simple text form.
	pub fn parse(text: &str) -> Result<Self, uuid::Error> {
		Uuid::parse_str(text).map(Self::from_uuid)
	}
}

/// Something that carries the key of a row of `T`: a table keyed by a
/// single `Id<Self>`, which is what an `Id<T>` column points at, or a client
/// type of one, such as `PublicUser`, which carries a user's key. The
/// macros implement it.
///
/// A list of these can be compared by key, which is how a client keeps
/// each row's state across updates.
pub trait HasId<T: ?Sized = Self> {
	/// The row's key.
	fn id(&self) -> Id<T>;
}

#[cfg(feature = "server")]
impl<T: crate::Table> Id<T> {
	/// The same ID every time for the same natural key, such as a wallet's
	/// chain and address: a UUIDv5 in a namespace derived from the table's
	/// name. Use it to upsert by natural key without looking the row up first.
	pub fn from_natural_key(parts: &[&str]) -> Self {
		let namespace = Uuid::new_v5(&Uuid::NAMESPACE_OID, T::flat_name().as_bytes());
		let mut name = Vec::new();

		// Length prefixes keep ("ab", "c") and ("a", "bc") apart.
		for part in parts {
			name.extend_from_slice(&(part.len() as u64).to_be_bytes());
			name.extend_from_slice(part.as_bytes());
		}

		Self::from_uuid(Uuid::new_v5(&namespace, &name))
	}
}

#[cfg(feature = "server")]
impl<T: ?Sized> Default for Id<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T: ?Sized> Clone for Id<T> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<T: ?Sized> Copy for Id<T> {}

impl<T: ?Sized> PartialEq for Id<T> {
	fn eq(&self, other: &Self) -> bool {
		self.uuid == other.uuid
	}
}

impl<T: ?Sized> Eq for Id<T> {}

impl<T: ?Sized> PartialOrd for Id<T> {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl<T: ?Sized> Ord for Id<T> {
	fn cmp(&self, other: &Self) -> Ordering {
		self.uuid.cmp(&other.uuid)
	}
}

impl<T: ?Sized> Hash for Id<T> {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.uuid.hash(state);
	}
}

impl<T: ?Sized> fmt::Debug for Id<T> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Id({})", self.uuid)
	}
}

impl<T: ?Sized> fmt::Display for Id<T> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		fmt::Display::fmt(&self.uuid, f)
	}
}

impl<T: ?Sized> FromStr for Id<T> {
	type Err = uuid::Error;

	fn from_str(text: &str) -> Result<Self, Self::Err> {
		Self::parse(text)
	}
}

impl<T: ?Sized> From<Id<T>> for Uuid {
	fn from(id: Id<T>) -> Self {
		id.uuid
	}
}

impl<T: ?Sized> serde::Serialize for Id<T> {
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		self.uuid.serialize(serializer)
	}
}

impl<'de, T: ?Sized> serde::Deserialize<'de> for Id<T> {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		Uuid::deserialize(deserializer).map(Self::from_uuid)
	}
}

impl<T: ?Sized> schemars::JsonSchema for Id<T> {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		"Id".into()
	}

	fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
		Uuid::json_schema(generator)
	}
}
