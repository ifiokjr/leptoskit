//! `#[api(scope = …)]`: the scopes a third-party token needs, registered
//! with the function and published in the spec.
#![cfg(feature = "ssr")]
// `#[server]` functions are async by contract, awaited or not.
#![allow(clippy::unused_async)]

use http::Method;
use leptos::prelude::*;
use leptoskit_rpc::Scopes;
use leptoskit_rpc::api;
use leptoskit_rpc::spec;
use leptoskit_rpc::spec::SpecOptions;

leptoskit_rpc::scopes! {
	/// What other apps may do.
	pub enum Scope {
		/// Read your posts.
		PostsRead = "posts:read",
		/// Edit your posts.
		PostsWrite = "posts:write",
	}
}

/// Lists the caller's posts.
#[api(scope = Scope::PostsRead)]
#[server]
pub async fn list_my_posts() -> Result<Vec<String>, ServerFnError> {
	Ok(Vec::new())
}

/// Edits a post.
#[api(scope = Scope::PostsRead, scope = "posts:write")]
#[server]
pub async fn edit_post(id: u64) -> Result<(), ServerFnError> {
	let _ = id;
	Ok(())
}

/// Only the app's own sessions call this.
#[api]
#[server]
pub async fn first_party_only() -> Result<(), ServerFnError> {
	Ok(())
}

#[test]
fn routes_name_the_scopes_they_require() {
	assert_eq!(
		spec::scopes_for(&Method::POST, "/api/list_my_posts"),
		Some(&["posts:read"][..])
	);
	assert_eq!(
		spec::scopes_for(&Method::POST, "/api/edit_post"),
		Some(&["posts:read", "posts:write"][..])
	);
	assert_eq!(
		spec::scopes_for(&Method::POST, "/api/first_party_only"),
		Some(&[][..])
	);
	assert_eq!(spec::scopes_for(&Method::GET, "/api/list_my_posts"), None);
	assert_eq!(spec::scopes_for(&Method::POST, "/api/nothing"), None);
	assert_eq!(Scope::ALL[1].name, Scope::PostsWrite.as_str());
}

#[test]
fn the_spec_and_openapi_carry_the_scopes() {
	let spec = spec::collect(&SpecOptions::new("posts", "1.0.0")).unwrap();
	let edit = spec
		.functions
		.iter()
		.find(|function| function.name == "edit_post")
		.unwrap();
	assert_eq!(edit.scopes, ["posts:read", "posts:write"]);

	let document = spec.to_openapi();
	assert_eq!(
		document["paths"]["/api/list_my_posts"]["post"]["security"][0]["oauth2"][0],
		"posts:read"
	);
	assert!(
		document["paths"]["/api/first_party_only"]["post"]
			.get("security")
			.is_none()
	);
}
