//! Two same-named functions in different modules land on one route, which
//! leptos would silently answer with only one of them: the server refuses to
//! start rather than lose a function.
#![cfg(feature = "ssr")]
// `#[server]` functions are async by contract, awaited or not.
#![allow(clippy::unused_async)]

use std::process::Command;

use leptoskit_rpc::spec;

mod todos {
	use leptos::prelude::*;
	use leptoskit_rpc::api;

	/// Lists todos.
	#[api]
	#[server]
	pub async fn list() -> Result<Vec<String>, ServerFnError> {
		Ok(Vec::new())
	}
}

mod notes {
	use leptos::prelude::*;
	use leptoskit_rpc::api;

	/// Lists notes.
	#[api]
	#[server]
	pub async fn list() -> Result<Vec<String>, ServerFnError> {
		Ok(Vec::new())
	}
}

/// Set in the child: the test body becomes the app's `main`.
const CHILD: &str = "LEPTOSKIT_RPC_DUPLICATE_CHILD";

#[test]
fn both_functions_share_one_route() {
	let paths: Vec<&str> = spec::descriptors()
		.iter()
		.map(|descriptor| descriptor.path)
		.collect();
	assert_eq!(paths, ["/api/list", "/api/list"]);
	let error = spec::check().expect_err("a duplicate");
	assert!(
		matches!(error, spec::SpecError::DuplicateName { ref name, .. } if name == "list"),
		"{error}"
	);
	assert!(
		error.to_string().contains("#[api(name = \"...\")]"),
		"{error}"
	);
}

#[test]
fn the_server_refuses_to_start() {
	if std::env::var_os(CHILD).is_some() {
		spec::export_if_requested();
		unreachable!("a duplicate exits before the server starts");
	}
	let child = Command::new(std::env::current_exe().expect("the test binary"))
		.args([
			"--exact",
			"the_server_refuses_to_start",
			"--nocapture",
			"--test-threads=1",
		])
		.env(CHILD, "1")
		.env_remove(spec::SPEC_OUT_ENV)
		.output()
		.expect("run the child");
	let stderr = String::from_utf8_lossy(&child.stderr);
	assert_eq!(child.status.code(), Some(1), "{stderr}");
	assert!(
		stderr.contains("two #[api] functions are named `list`"),
		"{stderr}"
	);
	assert!(
		stderr.contains("duplicate_routes::notes") && stderr.contains("duplicate_routes::todos"),
		"{stderr}"
	);
}
