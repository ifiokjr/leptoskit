//! JSON columns.

/// A value stored as JSON: `jsonb` on Postgres, checked text on SQLite. On the
/// wire it is the value itself, not wrapped.
#[derive(
	Clone,
	Debug,
	Default,
	PartialEq,
	Eq,
	Hash,
	serde::Serialize,
	serde::Deserialize,
	schemars::JsonSchema,
)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct Json<T>(pub T);

impl<T> std::ops::Deref for Json<T> {
	type Target = T;

	fn deref(&self) -> &T {
		&self.0
	}
}

impl<T> std::ops::DerefMut for Json<T> {
	fn deref_mut(&mut self) -> &mut T {
		&mut self.0
	}
}
