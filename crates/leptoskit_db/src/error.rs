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

	/// Whether a unique value was already taken.
	pub fn is_conflict(&self) -> bool {
		matches!(self, Self::Conflict { .. })
	}
}

/// The outcome of a database operation.
pub type Result<T, E = DbError> = std::result::Result<T, E>;
