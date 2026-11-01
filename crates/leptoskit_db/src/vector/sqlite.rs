//! sqlite-vec, linked into every connection, and the functions vector
//! columns need that it lacks, registered through `rusqlite`'s safe API.
//!
//! sqlite-vec ships as C compiled into the build, so nothing needs
//! installing on servers or devices. It measures Euclidean, taxicab,
//! cosine, and Hamming distances, and ranks by them in its `vec0` tables.
//! The inner product and Jaccard distance are measured here instead, as
//! pgvector measures them, and `lk_sha256` fingerprints the text a vector
//! was made from.

use std::ffi::c_char;
use std::ffi::c_int;

use rusqlite::Connection;
use rusqlite::ffi;
use rusqlite::functions::Context;
use rusqlite::functions::FunctionFlags;
use rusqlite::types::ValueRef;
use sha2::Digest;

/// sqlite-vec's entry point, as SQLite's extension loader calls it.
type Init = unsafe extern "C" fn(
	*mut ffi::sqlite3,
	*mut *mut c_char,
	*const ffi::sqlite3_api_routines,
) -> c_int;

/// Registers sqlite-vec and every function on `connection`.
pub(crate) fn register(connection: &Connection) -> rusqlite::Result<()> {
	load_sqlite_vec(connection)?;
	let flags = FunctionFlags::SQLITE_UTF8
		| FunctionFlags::SQLITE_DETERMINISTIC
		| FunctionFlags::SQLITE_INNOCUOUS;

	connection.create_scalar_function("lk_sha256", 1, flags, |context| {
		Ok(match context.get_raw(0) {
			ValueRef::Text(text) => Some(format!("{:x}", sha2::Sha256::digest(text))),
			_ => None,
		})
	})?;
	connection.create_scalar_function("lk_vec_distance_ip", 2, flags, |context| {
		let (Some(left), Some(right)) = (floats(context, 0)?, floats(context, 1)?) else {
			return Ok(None);
		};
		same_length(left.len(), right.len())?;
		let product: f64 = left
			.iter()
			.zip(&right)
			.map(|(left, right)| f64::from(*left) * f64::from(*right))
			.sum();
		Ok(Some(-product))
	})?;
	connection.create_scalar_function("lk_vec_distance_jaccard", 2, flags, |context| {
		let (Some(left), Some(right)) = (blob(context, 0)?, blob(context, 1)?) else {
			return Ok(None);
		};
		same_length(left.len(), right.len())?;
		Ok(Some(jaccard(left, right)))
	})?;
	Ok(())
}

/// Runs sqlite-vec's entry point on `connection`, which registers its
/// functions and its `vec0` module there.
#[allow(unsafe_code)]
fn load_sqlite_vec(connection: &Connection) -> rusqlite::Result<()> {
	let mut message: *mut c_char = std::ptr::null_mut();
	// SAFETY: `sqlite3_vec_init` is compiled into this binary against the
	// SQLite `rusqlite` links, with `SQLITE_CORE`, so it ignores the API
	// routines and has exactly `Init`'s signature, which the crate declares
	// without arguments. The handle is open for the call, and `message` is
	// a valid place for the error SQLite allocates, freed below.
	let status = unsafe {
		let init =
			std::mem::transmute::<*const (), Init>(sqlite_vec::sqlite3_vec_init as *const ());
		init(connection.handle(), &raw mut message, std::ptr::null())
	};

	if status == ffi::SQLITE_OK {
		return Ok(());
	}

	// SAFETY: when set, `message` is a NUL-terminated string SQLite
	// allocated, read once and freed with `sqlite3_free`.
	let text = unsafe {
		if message.is_null() {
			None
		} else {
			let text = std::ffi::CStr::from_ptr(message)
				.to_string_lossy()
				.into_owned();
			ffi::sqlite3_free(message.cast());
			Some(text)
		}
	};

	Err(rusqlite::Error::SqliteFailure(
		ffi::Error::new(status),
		text,
	))
}

/// The pgvector distance of two bit vectors' sets: one minus the share of
/// set bits they have in common, or 1 when neither has a bit set.
pub(crate) fn jaccard(left: &[u8], right: &[u8]) -> f64 {
	let (mut both, mut either) = (0_u32, 0_u32);

	for (left, right) in left.iter().zip(right) {
		both += (left & right).count_ones();
		either += (left | right).count_ones();
	}

	if either == 0 {
		return 1.0;
	}

	1.0 - f64::from(both) / f64::from(either)
}

/// Argument `index` as bytes, or `None` when it's `NULL`.
fn blob<'a>(context: &'a Context<'_>, index: usize) -> rusqlite::Result<Option<&'a [u8]>> {
	match context.get_raw(index) {
		ValueRef::Null => Ok(None),
		ValueRef::Blob(bytes) => Ok(Some(bytes)),
		_ => Err(user_error("a vector is a BLOB")),
	}
}

/// Argument `index` as floats in sqlite-vec's layout, or `None` when it's
/// `NULL`.
fn floats(context: &Context<'_>, index: usize) -> rusqlite::Result<Option<Vec<f32>>> {
	let Some(bytes) = blob(context, index)? else {
		return Ok(None);
	};

	if bytes.len() % 4 != 0 {
		return Err(user_error(
			"a float vector is a whole number of 4-byte floats",
		));
	}

	Ok(Some(
		bytes
			.as_chunks::<4>()
			.0
			.iter()
			.map(|chunk| f32::from_le_bytes(*chunk))
			.collect(),
	))
}

fn same_length(left: usize, right: usize) -> rusqlite::Result<()> {
	if left == right {
		Ok(())
	} else {
		Err(user_error("the vectors have different dimensions"))
	}
}

fn user_error(message: &str) -> rusqlite::Error {
	rusqlite::Error::UserFunctionError(message.into())
}
