//! The globals a build with no live connection has. Its own test binary,
//! since the process's live client, once installed, stays.

#![cfg(all(feature = "live", feature = "sqlite"))]

use leptos::prelude::Get as _;
use leptos::prelude::Owner;
use leptoskit_rpc as rpc;
use leptoskit_rpc::client::ClientConfig;

/// A build with no installed client reports no connection, and the globals
/// do nothing.
#[test]
fn without_an_installed_client_the_globals_are_offline() {
	rpc::client::configure(ClientConfig::new());
	assert!(!rpc::client::live::available());
	rpc::client::live::reauthenticate();
	// Signal hooks run inside a component owner, including builds using
	// sandboxed arenas.
	Owner::new().with(|| {
		let status = rpc::client::live::use_live_status();
		assert_eq!(status.get(), rpc::client::LiveStatus::OFFLINE);
	});
}
