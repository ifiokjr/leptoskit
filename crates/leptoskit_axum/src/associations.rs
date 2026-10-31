//! The files that associate the site with its native apps, generated from
//! `[native]` so they can't drift from the apps' IDs:
//!
//! - `/.well-known/apple-app-site-association` lists the iOS and macOS app
//!   under `webcredentials`, so it can use the site's passkeys, and under
//!   `applinks` for the sign-in return address.
//! - `/.well-known/assetlinks.json` lets the Android app, signed with the
//!   listed certificates, use the site's passkeys and open its links.

use axum::Router;
use axum::http::HeaderValue;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use leptoskit_config::NATIVE_AUTH_RETURN_PATH;
use leptoskit_config::NativeConfig;
use serde_json::Value;
use serde_json::json;

/// Serves `/.well-known/apple-app-site-association` when `[native]` has an
/// `apple_team_id`, and `/.well-known/assetlinks.json` when it has
/// `android_certificates`. Merge it into the app's router; serve it from the
/// site the apps sign in to, over `https`.
///
/// ```
/// let config = leptoskit_config::LeptoskitConfig::from_toml(
/// 	"[app]\nname = \"notes\"\n[native]\npackage_id = \"com.example.notes\"\napple_team_id = \
/// 	 \"ABCDE12345\"",
/// 	std::path::Path::new("notes"),
/// )
/// .unwrap();
/// let router: leptoskit_axum::axum::Router = leptoskit_axum::app_associations(&config.native);
/// ```
pub fn app_associations<S>(native: &NativeConfig) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let mut router = Router::new();

	if let Some(file) = apple_app_site_association(native) {
		let body = file.to_string();
		router = router.route(
			"/.well-known/apple-app-site-association",
			get(move || std::future::ready(json_file(body.clone()))),
		);
	}

	if let Some(file) = asset_links(native) {
		let body = file.to_string();
		router = router.route(
			"/.well-known/assetlinks.json",
			get(move || std::future::ready(json_file(body.clone()))),
		);
	}

	router
}

fn json_file(body: String) -> Response {
	let mut response = body.into_response();
	let headers = response.headers_mut();
	headers.insert(
		header::CONTENT_TYPE,
		HeaderValue::from_static("application/json"),
	);
	headers.insert(
		header::CACHE_CONTROL,
		HeaderValue::from_static("public, max-age=3600"),
	);
	response
}

/// The Apple app site association: the app (`<team>.<package id>`) may use
/// the site's passkeys, and claims the sign-in return address. `None`
/// without an `apple_team_id`.
pub fn apple_app_site_association(native: &NativeConfig) -> Option<Value> {
	let team = native.apple_team_id.as_deref()?;

	if native.package_id.is_empty() {
		return None;
	}

	let app = format!("{team}.{}", native.package_id);
	Some(json!({
		"applinks": {
			"details": [{
				"appIDs": [app],
				"components": [{
					"/": NATIVE_AUTH_RETURN_PATH,
					"comment": "Where sign-in in the system browser returns to the app",
				}],
			}],
		},
		"webcredentials": { "apps": [app] },
	}))
}

/// The Digital Asset Links statement: the Android app, signed with the
/// listed certificates, may use the site's passkeys and handle its links.
/// `None` without `android_certificates`.
pub fn asset_links(native: &NativeConfig) -> Option<Value> {
	let fingerprints: Vec<String> = native
		.android_certificate_digests()
		.iter()
		.map(|digest| {
			digest
				.iter()
				.map(|byte| format!("{byte:02X}"))
				.collect::<Vec<_>>()
				.join(":")
		})
		.collect();

	if fingerprints.is_empty() || native.package_id.is_empty() {
		return None;
	}

	Some(json!([{
		"relation": [
			"delegate_permission/common.handle_all_urls",
			"delegate_permission/common.get_login_creds",
		],
		"target": {
			"namespace": "android_app",
			"package_name": native.package_id,
			"sha256_cert_fingerprints": fingerprints,
		},
	}]))
}
