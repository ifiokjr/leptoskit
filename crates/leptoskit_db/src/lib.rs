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
mod audit;
#[cfg(feature = "server")]
mod cascade;
#[cfg(feature = "server")]
mod connection;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod db;
#[cfg(feature = "server")]
mod error;
#[cfg(feature = "server")]
mod eval;
#[cfg(feature = "server")]
mod expr;
#[cfg(feature = "server")]
mod hooks;
mod id;
mod json;
#[cfg(feature = "server")]
mod links;
#[cfg(feature = "server")]
mod live;
#[cfg(feature = "server")]
mod migrate;
mod mixins;
#[cfg(feature = "server")]
mod policy;
#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "server")]
mod query;
#[cfg(feature = "server")]
mod read_set;
mod scalars;
#[cfg(feature = "server")]
mod shape;
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
mod view;
#[cfg(feature = "server")]
mod write;

#[cfg(feature = "server")]
pub use audit::AuditEntry;
#[cfg(feature = "server")]
pub use audit::AuditEntryColumns;
#[cfg(feature = "server")]
pub use audit::AuditOp;
#[cfg(feature = "server")]
pub use audit::AuditReader;
#[cfg(feature = "server")]
pub use connection::Connection;
#[cfg(feature = "server")]
pub use connection::PoolOptions;
#[cfg(feature = "server")]
pub use connection::Read;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use db::Db;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use db::Tx;
#[cfg(feature = "server")]
pub use error::DbError;
#[cfg(feature = "server")]
pub use error::Result;
#[cfg(feature = "server")]
pub use eval::Row;
#[cfg(feature = "server")]
pub use eval::Tri;
#[cfg(feature = "server")]
pub use expr::Col;
#[cfg(feature = "server")]
#[doc(hidden)]
pub use expr::Comparison;
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
#[cfg(feature = "server")]
pub use hooks::Actor;
#[cfg(feature = "server")]
pub use hooks::ActorColumn;
#[cfg(feature = "server")]
pub use hooks::AfterDelete;
#[cfg(feature = "server")]
pub use hooks::AfterInsert;
#[cfg(feature = "server")]
pub use hooks::AfterUpdate;
#[cfg(feature = "server")]
pub use hooks::BeforeWrite;
#[cfg(feature = "server")]
pub use hooks::Hooks;
#[cfg(feature = "server")]
pub use hooks::WriteContext;
pub use id::HasId;
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
/// Says what to fetch about a row and the rows linked to it, fetched in one
/// statement with every table's policies applied. See [`Shape`] and the
/// guide's "Fetch linked rows in one query".
pub use leptoskit_db_macros::Shape;
/// Declares a table that links two others many to many. The struct holds
/// the link's own properties; the macro adds the two ends, which together
/// are the key, and deletes a link when either end is deleted. Write it as
/// `db::link`: a bare `#[link]` is Rust's own attribute for native
/// libraries.
///
/// ```ignore
/// /// A tag on a post.
/// #[db::link(name = "post_tags", from = Post as tags, to = Tag as posts)]
/// pub struct PostTag {
/// 	pub position: i16,
/// 	#[db(created_at)]
/// 	pub added_at: Timestamp,
/// }
///
/// post.tags().add(&db, tag.id, PostTagProperties { position: 0 }).await?;
/// let tags: Vec<(Tag, PostTag)> = post.tags().order_by(|link| link.position.asc()).all(&db).await?;
/// let posts: u64 = tag.posts().count(&db).await?;
/// ```
///
/// A link is a table like any other: it takes the same options as
/// `#[db::table]` and its policies, and `exclusive` lets each `to` row be
/// linked from one `from` row only. A table linking to itself names its
/// columns: `from_column = "follower_id", to_column = "followed_id"`.
pub use leptoskit_db_macros::link;
/// Embeds the migrations in a directory, in `registry.txt` order:
/// `static MIGRATIONS: &[Migration] = migrations!("../migrations");`. The path
/// is relative to the crate's `Cargo.toml`, and the build fails if a
/// migration is missing or unlisted.
pub use leptoskit_db_macros::migrations;
/// Declares a set of columns once, for any table to flatten in:
///
/// ```ignore
/// #[db::mixin]
/// pub struct Address {
/// 	#[db(max_len = 200)]
/// 	pub street: String,
/// 	pub city: String,
/// }
///
/// #[db::table(name = "orders", public)]
/// pub struct Order {
/// 	pub id: Id<Order>,
/// 	#[db(flatten, prefix = "billing_")]
/// 	pub billing: Address,
/// 	#[db(flatten)]
/// 	pub times: db::Timestamps,
/// }
/// ```
///
/// The columns keep their names, after the prefix: `billing_street`. A mixin's
/// fields take the same `#[db(...)]` options as a table's, and its own
/// `#[db::mixin(index(...), unique(...))]` rules follow it into every table.
pub use leptoskit_db_macros::mixin;
/// Declares a rule of a table's access policy. Write it below
/// `#[db::table]`, which reads it:
///
/// ```ignore
/// #[db::table(name = "posts", public)]
/// #[db::policy(select, |post, cx| post.published.eq(true) | cx.global(|viewer: &Viewer| post.author_id.eq(viewer.0)))]
/// #[db::policy(insert, update, delete, |post, cx| cx.global(|viewer: &Viewer| post.author_id.eq(viewer.0)),
/// 	message = "Only the author can change a post")]
/// pub struct Post { /* … */ }
/// ```
///
/// Once a table has a policy, only what a rule allows is allowed and a `deny`
/// wins over every `allow`. Reads, changes, and deletes skip rows silently;
/// a refused insert, or a change to something not allowed, fails with
/// `DbError::Forbidden`. `Db::system` bypasses policies, explicitly.
pub use leptoskit_db_macros::policy;
/// Declares a table. See the crate documentation.
pub use leptoskit_db_macros::table;
/// Declares a read-only view: a query the migrations create under a name,
/// read like a table. The struct's fields are the query's columns, in
/// order.
///
/// ```ignore
/// /// How much each author has written.
/// #[db::view(name = "author_stats", sql = "
/// 	SELECT author_id, count(*) AS posts FROM posts WHERE deleted_at IS NULL GROUP BY author_id")]
/// pub struct AuthorStats {
/// 	pub author_id: Id<User>,
/// 	pub posts: i64,
/// }
///
/// let busy = AuthorStats::query().filter(|s| s.posts.gt(10)).all(&db).await?;
/// ```
///
/// `materialized` keeps the rows its query returned until
/// `AuthorStats::refresh(&db)` runs it again. `sql(postgres = "…", sqlite =
/// "…")` gives each dialect its own query. Its `#[db::policy(select, …)]`
/// rules apply as a table's do; the query itself reads every row of its
/// tables, whatever their policies.
pub use leptoskit_db_macros::view;
#[cfg(feature = "server")]
pub use leptoskit_db_schema::Dialect;
#[cfg(feature = "server")]
pub use leptoskit_db_schema::TableName;
#[cfg(feature = "server")]
pub use links::Link;
#[cfg(feature = "server")]
pub use links::Links;
#[cfg(feature = "server")]
pub use links::WritableLink;
#[cfg(feature = "server")]
pub use live::Change;
#[cfg(feature = "server")]
pub use live::ChangeOp;
#[cfg(feature = "server")]
pub use live::Changes;
#[cfg(feature = "server")]
pub use live::Commit;
#[cfg(feature = "server")]
pub use live::latest_version;
#[cfg(feature = "server")]
pub use live::prune as prune_live;
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
pub use mixins::HasMixin;
pub use mixins::Mixin;
#[doc(hidden)]
pub use mixins::MixinField;
pub use mixins::SoftDelete;
pub use mixins::Timestamps;
pub use mixins::Versioned;
#[cfg(feature = "server")]
pub use policy::Action;
#[cfg(feature = "server")]
pub use policy::Context;
#[cfg(feature = "server")]
pub use policy::Count;
#[cfg(feature = "server")]
pub use policy::Policy;
#[cfg(feature = "server")]
pub use query::Query;
#[cfg(feature = "server")]
pub use query::Update;
#[cfg(feature = "server")]
pub use read_set::RawRead;
#[cfg(feature = "server")]
pub use read_set::ReadEntry;
#[cfg(feature = "server")]
pub use read_set::ReadKind;
#[cfg(feature = "server")]
#[doc(hidden)]
pub use read_set::Reads;
#[cfg(feature = "server")]
pub use read_set::Tracked;
pub use scalars::Email;
pub use scalars::Slug;
pub use scalars::Url;
#[cfg(feature = "server")]
pub use shape::PublicShape;
#[cfg(feature = "server")]
pub use shape::Shape;
#[cfg(feature = "server")]
pub use shape::Shaped;
#[cfg(feature = "server")]
pub use sql::Statement;
#[cfg(feature = "server")]
pub use table::DIALECT_ENV;
#[cfg(feature = "server")]
pub use table::LiveTable;
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
#[cfg(feature = "server")]
pub use view::View;

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

/// Adds schema packs to the app: crates, such as `leptoskit_auth`, that ship
/// their own tables. Put it in the crate that declares the app's tables:
///
/// ```ignore
/// leptoskit::db::packs!(leptoskit_auth);
/// ```
///
/// The packs' tables then join the app's schema, and its migrations, like
/// its own. A dependency nothing refers to isn't linked into the program, so
/// without this its tables would silently vanish from the schema, and the
/// next migration would drop them.
#[macro_export]
macro_rules! packs {
	($($pack:ident),* $(,)?) => {
		$(extern crate $pack as _;)*
	};
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
	pub use leptoskit_db_macros::table_continue;
	#[cfg(feature = "server")]
	pub use leptoskit_db_schema as schema;
	pub use schemars;
	pub use serde;

	#[cfg(feature = "server")]
	pub use crate::cascade::SoftCascade;
	#[cfg(feature = "server")]
	pub use crate::table::TableBuilder;
	#[cfg(feature = "server")]
	pub use crate::table::TableEntry;
	#[cfg(feature = "server")]
	pub use crate::table::decode_next;
	#[cfg(feature = "server")]
	pub use crate::view::ViewEntry;
	#[cfg(feature = "server")]
	pub use crate::view::refresh;
	#[cfg(feature = "server")]
	pub use crate::view::view_definition;
	/// The link lookups generated accessors call.
	#[cfg(feature = "server")]
	pub mod link {
		pub use crate::links::one;
		pub use crate::links::optional;
	}
	/// What `#[derive(Shape)]` and the relations it follows expand to.
	#[cfg(feature = "server")]
	pub mod shape {
		pub use crate::shape::Many;
		pub use crate::shape::MaybeOne;
		pub use crate::shape::One;
		pub use crate::shape::Related;
		pub use crate::shape::Relation;
		pub use crate::shape::Selected;
		pub use crate::shape::decode_column;
		pub use crate::shape::decode_many;
		pub use crate::shape::decode_one;
		pub use crate::shape::decode_optional;
	}
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
