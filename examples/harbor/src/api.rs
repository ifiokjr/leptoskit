//! The harbour's RPC: leptos server functions, published with `#[api]` so the
//! spec and the generated clients describe them.
//!
//! This example builds in server mode, so calling one of these runs its body
//! in-process, as the server does for a request. A browser build of the same
//! functions calls them over HTTP, on the routes the spec lists.

use leptos::prelude::ServerFnError;
use leptos::server;
use leptoskit::rpc::api;

use crate::harbor::BookingError;
use crate::harbor::HARBOUR;
use crate::harbor::LATENCY;
use crate::harbor::LogPage;
use crate::harbor::Sailing;

/// The day's sailings, earliest first.
///
/// # Arguments
///
/// * `day` - The day of the week, from 1.
#[api(crate = "leptoskit::rpc", tag = "board")]
#[server(endpoint = "sailings")]
pub async fn sailings(day: u16) -> Result<Vec<Sailing>, ServerFnError> {
	tokio::time::sleep(LATENCY).await;
	Ok(HARBOUR.lock().sailings(day))
}

/// Books one seat on a sailing, and returns the sailing as it now stands.
///
/// Fails with `sold out` when every seat is taken, and `departed` once the
/// ferry has left.
///
/// # Arguments
///
/// * `sailing` - The sailing's id.
#[api(crate = "leptoskit::rpc", tag = "bookings")]
#[server(endpoint = "book_seat")]
pub async fn book_seat(sailing: u32) -> Result<Sailing, ServerFnError> {
	tokio::time::sleep(LATENCY).await;
	HARBOUR.lock().book(sailing).map_err(|error| {
		ServerFnError::new(match error {
			BookingError::Unknown => "no such sailing",
			BookingError::SoldOut => "sold out",
			BookingError::Departed => "departed",
		})
	})
}

/// The harbour master's log, newest first, a page at a time.
///
/// # Arguments
///
/// * `cursor` - Where the page ends: zero for the newest entries, then each
///   page's `next`.
#[api(crate = "leptoskit::rpc", tag = "log")]
#[server(endpoint = "harbor_log")]
pub async fn harbor_log(cursor: u32) -> Result<LogPage, ServerFnError> {
	tokio::time::sleep(LATENCY).await;
	Ok(HARBOUR.lock().log_page(cursor))
}
