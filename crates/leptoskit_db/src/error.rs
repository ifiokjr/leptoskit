//! What can go wrong talking to the database.

use std::error::Error;

use crate::DecodeError;
use crate::EncodeError;
use crate::ValidationErrors;

type Source = Box<dyn Error + Send + Sync>;

/// A database operation that failed.
///
/// `Display` names the table and the kind of failure, never SQL or values, so
/// that `?` into a server function's error can't leak data. The driver's error
/// is the `source`, for logs.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
	/// No row matched.
	#[error("no `{table}` row matched")]
	NotFound {
		/// The table that was searched.
		table: String,
	},
	/// A policy refused a write.
	#[error("{message}")]
	Forbidden {
		/// The table.
		table: String,
		/// Why: the policy's message, or a generic one.
		message: String,
	},
	/// A versioned row changed since it was read: another write saved first.
	#[error("the `{table}` row changed since it was read; read it again and retry")]
	Stale {
		/// The table.
		table: String,
	},
	/// A unique key already has the value.
	#[error("a `{table}` row with the same unique value already exists")]
	Conflict {
		/// The table, when the database reports it.
		table: String,
		/// The constraint or index, when the database reports it.
		constraint: Option<String>,
		#[source]
		source: Source,
	},
	/// A foreign key points at a missing row, or a deleted row is still
	/// referenced.
	#[error("a `{table}` row references a row that doesn't exist, or is still referenced")]
	ForeignKey {
		/// The table, when the database reports it.
		table: String,
		/// The constraint, when the database reports it.
		constraint: Option<String>,
		#[source]
		source: Source,
	},
	/// A check constraint failed.
	#[error("a value in `{table}` breaks a rule of the table")]
	Check {
		/// The table, when the database reports it.
		table: String,
		/// The constraint, when the database reports it.
		constraint: Option<String>,
		#[source]
		source: Source,
	},
	/// A bulk write would skip hooks the table declares, because it never
	/// loads the rows it changes. Call `allow_skipping_hooks()` on the bulk
	/// write to skip them on purpose, or write the rows one at a time.
	#[error(
		"`{table}` has hooks that a bulk write can't run; write the rows one at a time, or call \
		 `allow_skipping_hooks()`"
	)]
	HooksSkipped {
		/// The table.
		table: String,
	},
	/// A write records who made it in a required `created_by` or
	/// `updated_by` column, and nobody is acting: the request has no
	/// [`Actor`](crate::Actor).
	#[error("a `{table}` row records who wrote it in `{column}`, and nobody is acting")]
	NoActor {
		/// The table.
		table: String,
		/// The column that needs someone acting.
		column: String,
	},
	/// A view, or a column the database layer fills in, can't be written.
	#[error("{message}")]
	ReadOnly {
		/// The view or table.
		table: String,
		/// What can't be written, and why.
		message: String,
	},
	/// A connection that records what it reads, a
	/// [`Tracked`](crate::Tracked) one, was asked to run SQL that doesn't say
	/// which tables it reads, so a live query couldn't tell when its result
	/// changes. Raw SQL declares them with
	/// [`Statement::reads`](crate::Statement::reads), and a view with
	/// `#[db::view(reads(…))]`.
	#[error("{}", undeclared(table.as_deref()))]
	UndeclaredRead {
		/// The view that doesn't declare what it reads, or `None` for raw
		/// SQL.
		table: Option<String>,
	},
	/// A row broke the rules its columns declare, before reaching the database.
	#[error(transparent)]
	Invalid(#[from] ValidationErrors),
	/// A value couldn't be stored.
	#[error("a value can't be stored")]
	Encode(#[from] EncodeError),
	/// A stored value isn't what the Rust type expects.
	#[error("a stored value can't be read")]
	Decode(#[from] DecodeError),
	/// The database rejected or failed the query for another reason.
	#[error("the database couldn't run a query")]
	Query(#[source] Source),
	/// The database couldn't be reached.
	#[error("the database can't be reached")]
	Connection(#[source] Source),
}

/// What an [`DbError::UndeclaredRead`] says.
fn undeclared(table: Option<&str>) -> String {
	match table {
		Some(view) => {
			format!(
				"the view `{view}` doesn't declare the tables it reads, so a tracked read can't \
				 record it; add `reads(…)` to its `#[db::view]`"
			)
		}
		None => {
			"a tracked read can't run SQL that doesn't declare the tables it reads; use \
			 `Statement::reads`"
				.to_string()
		}
	}
}

impl DbError {
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	pub(crate) fn query(error: impl Error + Send + Sync + 'static) -> Self {
		Self::Query(Box::new(error))
	}

	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	pub(crate) fn connection(error: impl Error + Send + Sync + 'static) -> Self {
		Self::Connection(Box::new(error))
	}

	/// Whether no row matched.
	pub fn is_not_found(&self) -> bool {
		matches!(self, Self::NotFound { .. })
	}

	/// Whether a policy refused a write.
	pub fn is_forbidden(&self) -> bool {
		matches!(self, Self::Forbidden { .. })
	}

	/// Whether a versioned row changed since it was read.
	pub fn is_stale(&self) -> bool {
		matches!(self, Self::Stale { .. })
	}

	/// Whether a unique value was already taken.
	pub fn is_conflict(&self) -> bool {
		matches!(self, Self::Conflict { .. })
	}

	/// Whether the database gave up on the transaction because another one
	/// got in its way: a serialization failure (`40001`) or a deadlock
	/// (`40P01`) on Postgres, or a busy or locked database on SQLite. Running
	/// the whole transaction again can succeed.
	pub fn is_contended(&self) -> bool {
		let (Self::Query(source) | Self::Connection(source)) = self else {
			return false;
		};

		#[cfg(feature = "postgres")]
		if let Some(code) = source
			.downcast_ref::<tokio_postgres::Error>()
			.and_then(tokio_postgres::Error::code)
		{
			use tokio_postgres::error::SqlState;

			return *code == SqlState::T_R_SERIALIZATION_FAILURE
				|| *code == SqlState::T_R_DEADLOCK_DETECTED;
		}

		#[cfg(feature = "sqlite")]
		if let Some(rusqlite::Error::SqliteFailure(code, _)) =
			source.downcast_ref::<rusqlite::Error>()
		{
			return matches!(
				code.code,
				rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
			);
		}

		let _ = source;
		false
	}
}

/// The outcome of a database operation.
pub type Result<T, E = DbError> = std::result::Result<T, E>;
