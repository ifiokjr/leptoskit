//! leptoskit's database layer: tables declared as Rust structs, typed IDs,
//! and client-safe public projections.
//!
//! ```ignore
//! use leptoskit::db;
//! use leptoskit::db::Id;
//! use leptoskit::db::Timestamp;
//!
//! /// Someone with an account.
//! #[db::table(name = "users", public)]
//! pub struct User {
//! 	pub id: Id<User>,
//! 	#[db(unique, max_len = 32)]
//! 	pub handle: String,
//! 	#[db(private)]
//! 	pub email: String,
//! 	#[db(secret)]
//! 	pub password_hash: String,
//! 	#[db(created_at)]
//! 	pub created_at: Timestamp,
//! }
//! ```
//!
//! On the server (the `server` feature), that declares the `User` row, a
//! `NewUser` to insert, and the table's schema, which `leptoskit db` turns
//! into migrations. Everywhere, it declares `PublicUser`: every column except
//! the `private` and `secret` ones, serializable, and the only form of a user
//! a server function can return. On the client, `User` is an empty marker, so
//! `Id<User>` still works but no row can be built or read there.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

// Lets `#[derive(Scalar)]` name this crate from inside it, as it names it in
// any other crate.
extern crate self as leptoskit_db;

#[cfg(feature = "server")]
mod connection;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod db;
#[cfg(feature = "server")]
mod error;
#[cfg(feature = "server")]
mod expr;
mod id;
mod json;
#[cfg(feature = "server")]
mod migrate;
#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "server")]
mod query;
mod scalars;
#[cfg(feature = "server")]
mod sql;
#[cfg(feature = "sqlite")]
mod sqlite;
#[cfg(feature = "server")]
mod table;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub mod testing;
mod time;
pub mod validate;
#[cfg(feature = "server")]
mod value;
#[cfg(feature = "server")]
mod write;

#[cfg(feature = "server")]
pub use connection::Connection;
#[cfg(feature = "server")]
pub use connection::PoolOptions;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use db::Db;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use db::Tx;
#[cfg(feature = "server")]
pub use error::DbError;
#[cfg(feature = "server")]
pub use error::Result;
#[cfg(feature = "server")]
pub use expr::Col;
#[cfg(feature = "server")]
pub use expr::ConflictTarget;
#[cfg(feature = "server")]
pub use expr::Expr;
#[cfg(feature = "server")]
pub use expr::Operand;
#[cfg(feature = "server")]
pub use expr::Order;
#[cfg(feature = "server")]
pub use expr::TextColumn;
pub use id::Id;
pub use json::Json;
/// Stores a unit enum as text, one value per variant: the variant's name in
/// `snake_case`, or `#[db(rename = "...")]`. Generates `as_str`, `Display`,
/// `FromStr`, serde, and JSON Schema impls, and a check that the column
/// holds only those values.
pub use leptoskit_db_macros::Enum;
/// A newtype that is valid by construction: `parse` normalizes and checks
/// it, `Deserialize` goes through `parse`, its JSON Schema carries its rules,
/// and every column of its type gets them as database checks. See
/// [`Email`] for an example.
///
/// ```ignore
/// #[derive(db::Scalar, Clone, Debug, PartialEq, Eq, Hash)]
/// #[db(min_len = 4, max_len = 20, regex = "^[a-z0-9_]+$", normalize(trim, lowercase))]
/// pub struct Handle(String);
///
/// let handle = Handle::parse(" Ifiok_Jr ")?; // "ifiok_jr"
/// ```
pub use leptoskit_db_macros::Scalar;
/// Embeds the migrations in a directory, in `registry.txt` order:
/// `static MIGRATIONS: &[Migration] = migrations!("../migrations");`. The path
/// is relative to the crate's `Cargo.toml`, and the build fails if a
/// migration is missing or unlisted.
pub use leptoskit_db_macros::migrations;
/// Declares a table. See the crate documentation.
pub use leptoskit_db_macros::table;
#[cfg(feature = "server")]
pub use leptoskit_db_schema::Dialect;
#[cfg(feature = "server")]
pub use migrate::MIGRATIONS_TABLE;
#[cfg(feature = "server")]
pub use migrate::Migration;
#[cfg(feature = "server")]
pub use migrate::MigrationError;
#[cfg(feature = "server")]
pub use migrate::checksum;
#[cfg(feature = "server")]
pub use migrate::load as load_migrations;
#[cfg(feature = "server")]
pub use migrate::registry_names;
#[cfg(feature = "server")]
pub use query::Query;
#[cfg(feature = "server")]
pub use query::Update;
pub use scalars::Email;
pub use scalars::Slug;
pub use scalars::Url;
#[cfg(feature = "server")]
pub use sql::Statement;
#[cfg(feature = "server")]
pub use table::DIALECT_ENV;
#[cfg(feature = "server")]
pub use table::SCHEMA_OUT_ENV;
#[cfg(feature = "server")]
pub use table::Table;
#[cfg(feature = "server")]
pub use table::schema;
pub use time::Timestamp;
#[cfg(feature = "server")]
pub use time::now;
pub use validate::ValidationErrors;
#[cfg(feature = "server")]
pub use value::Column;
#[cfg(feature = "server")]
pub use value::DecodeError;
#[cfg(feature = "server")]
pub use value::EncodeError;
#[cfg(feature = "server")]
pub use value::Reference;
#[cfg(feature = "server")]
pub use value::ScalarRules;
#[cfg(feature = "server")]
pub use value::Value;

/// A database enum's text didn't name a variant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{value}` isn't a {enumeration}; expected `{expected}`")]
pub struct UnknownVariant {
	/// The enum's name.
	pub enumeration: &'static str,
	/// The text that matched nothing.
	pub value: String,
	/// Every accepted value.
	pub expected: &'static str,
}

/// Lets `leptoskit db` read this crate's tables. Put it once in the crate that
/// declares them, such as `src/lib.rs`:
///
/// ```ignore
/// leptoskit::db::export_schema!();
/// ```
///
/// It adds a test, compiled only with the `server` feature, that does nothing
/// unless `leptoskit db` runs it to export the schema.
#[macro_export]
macro_rules! export_schema {
	() => {
		$crate::__server! {
			#[cfg(test)]
			#[test]
			fn __leptoskit_db_export_schema() {
				$crate::__private::export_schema_if_requested();
			}
		}
	};
}

/// Keeps its input only in server builds, so that generated rows and schema
/// code never reach a client.
#[cfg(feature = "server")]
#[doc(hidden)]
#[macro_export]
macro_rules! __server {
	($($tokens:tt)*) => { $($tokens)* };
}

#[cfg(not(feature = "server"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __server {
	($($tokens:tt)*) => {};
}

/// Keeps its input only in client builds.
#[cfg(feature = "server")]
#[doc(hidden)]
#[macro_export]
macro_rules! __client {
	($($tokens:tt)*) => {};
}

#[cfg(not(feature = "server"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __client {
	($($tokens:tt)*) => { $($tokens)* };
}

/// What generated code reaches for. Not part of the public API.
#[doc(hidden)]
pub mod __private {
	#[cfg(feature = "server")]
	pub use inventory;
	#[cfg(feature = "server")]
	pub use leptoskit_db_schema as schema;
	pub use schemars;
	pub use serde;

	#[cfg(feature = "server")]
	pub use crate::table::TableBuilder;
	#[cfg(feature = "server")]
	pub use crate::table::TableEntry;
	#[cfg(feature = "server")]
	pub use crate::table::decode_next;
	#[cfg(feature = "server")]
	pub use crate::table::export_schema_if_requested;
	pub use crate::validate::rules;
	/// The writes generated methods call.
	#[cfg(feature = "server")]
	pub mod write {
		pub use crate::write::delete;
		pub use crate::write::destroy;
		pub use crate::write::insert;
		pub use crate::write::save;
		pub use crate::write::upsert;
	}
}
