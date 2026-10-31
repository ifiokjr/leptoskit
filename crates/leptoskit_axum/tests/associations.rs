//! The native apps' association files, from `[native]`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use http_body_util::BodyExt;
use leptoskit_axum::app_associations;
use leptoskit_config::LeptoskitConfig;
use serde_json::Value;
use serde_json::json;
use tower::util::ServiceExt;

const FINGERPRINT: &str = "ab:cd:ef:01:23:45:67:89:ab:cd:ef:01:23:45:67:89:ab:cd:ef:01:23:45:67:\
                           89:ab:cd:ef:01:23:45:67:89";

fn config(native: &str) -> LeptoskitConfig {
	LeptoskitConfig::from_toml(
		&format!(
			"[app]\nname = \"notes\"\n\n[native]\npackage_id = \"com.example.notes\"\n{native}"
		),
		std::path::Path::new("notes"),
	)
	.unwrap()
}

async fn get(router: &axum::Router, path: &str) -> (StatusCode, Option<String>, Value) {
	let response = router
		.clone()
		.oneshot(Request::get(path).body(Body::empty()).unwrap())
		.await
		.unwrap();
	let status = response.status();
	let kind = response
		.headers()
		.get("content-type")
		.map(|value| value.to_str().unwrap().to_owned());
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	(
		status,
		kind,
		serde_json::from_slice(&bytes).unwrap_or(Value::Null),
	)
}

#[tokio::test]
async fn the_site_vouches_for_its_own_apps() {
	let config = config(&format!(
		"apple_team_id = \"ABCDE12345\"\nandroid_certificates = [\"{FINGERPRINT}\"]"
	));
	let router = app_associations(&config.native);

	let (status, kind, apple) = get(&router, "/.well-known/apple-app-site-association").await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(kind.as_deref(), Some("application/json"));
	assert_eq!(
		apple,
		json!({
			"applinks": { "details": [{
				"appIDs": ["ABCDE12345.com.example.notes"],
				"components": [{
					"/": "/auth/native/return",
					"comment": "Where sign-in in the system browser returns to the app",
				}],
			}]},
			"webcredentials": { "apps": ["ABCDE12345.com.example.notes"] },
		})
	);

	let (status, _, android) = get(&router, "/.well-known/assetlinks.json").await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(
		android[0]["relation"],
		json!([
			"delegate_permission/common.handle_all_urls",
			"delegate_permission/common.get_login_creds",
		])
	);
	assert_eq!(android[0]["target"]["package_name"], "com.example.notes");
	assert_eq!(
		android[0]["target"]["sha256_cert_fingerprints"],
		json!([FINGERPRINT.to_uppercase()])
	);
}

#[tokio::test]
async fn nothing_is_published_for_apps_the_config_does_not_describe() {
	let router = app_associations(&config("").native);

	for path in [
		"/.well-known/apple-app-site-association",
		"/.well-known/assetlinks.json",
	] {
		assert_eq!(get(&router, path).await.0, StatusCode::NOT_FOUND, "{path}");
	}

	let mut native = config("apple_team_id = \"ABCDE12345\"").native;
	native.package_id.clear();
	assert_eq!(leptoskit_axum::apple_app_site_association(&native), None);
	native.android_certificates = vec![FINGERPRINT.into()];
	assert_eq!(leptoskit_axum::asset_links(&native), None);
}
