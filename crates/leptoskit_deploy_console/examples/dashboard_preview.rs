//! Export the actual dashboard renderer with explicitly labeled example data.
use axum::body::Body;
use axum::body::to_bytes;
use axum::http::Request;
use leptoskit::analytics::*;
use leptoskit_deploy_console::control::*;
use tower::ServiceExt as _;

fn clock() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let directory = tempfile::tempdir()?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
	}
	let store = Store::open(&directory.path().join("preview.sqlite3"))?;
	store.register(&Project {
		id: "fieldnotes-preview".into(),
		environment: "example-data".into(),
		root: directory.path().canonicalize()?,
		revision: 0,
	})?;
	let operation = store.enqueue(
		"fieldnotes-preview",
		&Intent {
			expected_revision: 0,
			idempotency_key: "preview-release".into(),
			action: Action::Deploy {
				version: "preview-v1".into(),
				setup: false,
				maintenance: false,
			},
		},
	)?;
	store.claim()?;
	store.progress(&operation.id, "Verified the example image and schema")?;
	store.finish(
		&operation.id,
		&RunResult {
			succeeded: true,
			message: "Example release completed. This export uses synthetic measurements.".into(),
			details: serde_json::json!({"code":"preview"}),
		},
	)?;
	let analytics = Analytics::new(
		Resource {
			service: "fieldnotes-preview".into(),
			environment: "example-data".into(),
			release: "preview-v1".into(),
			instance: "example-one".into(),
		},
		Limits::default(),
		clock,
	)?;
	for index in 0..120 {
		analytics.record(
			SeriesKey {
				source: Source::Backend,
				signal: Signal::Http,
				operation: "/api/notes".into(),
				outcome: if index % 30 == 0 {
					Outcome::ServerError
				} else {
					Outcome::Ok
				},
			},
			10.0 + f64::from(index % 12),
			None,
		);
	}
	store.ingest("fieldnotes-preview", &analytics.snapshot())?;
	let key = "example-only-dashboard-render-key-0123456789";
	let service = Service::new(store, key, "http://localhost:4700")?;
	let response = service
		.router()
		.oneshot(
			Request::get("/")
				.header("authorization", format!("Bearer {key}"))
				.body(Body::empty())?,
		)
		.await?;
	let html = String::from_utf8(
		to_bytes(response.into_body(), 2 * 1024 * 1024)
			.await?
			.to_vec(),
	)?;
	// Keep the export self-contained; live actions are served by `lk console
	// serve`.
	let html = html
		.replace("<script type=\"module\" src=\"/console.mjs\"></script>", "")
		.replace(
			"background:#f8fafc;color:#0f172a",
			"background:transparent;color:var(--foreground,#0f172a)",
		)
		.replace(
			"max-width:1100px;margin:auto;padding:24px",
			"max-width:1100px;margin:auto;padding:0",
		)
		.replace(
			"background:#020617;color:#f1f5f9",
			"background:transparent;color:var(--foreground,#f1f5f9)",
		);
	let path = std::env::args()
		.nth(1)
		.unwrap_or_else(|| "target/deployment-dashboard.html".into());
	std::fs::write(&path, html)?;
	println!("{path}");
	Ok(())
}
