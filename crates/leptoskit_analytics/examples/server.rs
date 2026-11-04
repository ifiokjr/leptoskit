//! Run locally with a separate read key; never embed it in browser code.
use axum::Router;
use axum::routing::get;
use leptoskit_analytics::Limits;
use leptoskit_analytics::Resource;
use leptoskit_analytics::server::ServerConfig;
use leptoskit_analytics::server::{self};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let key = std::env::var("LEPTOSKIT_ANALYTICS_READ_KEY")?;
	let (app, _) = server::mount(
		Router::new().route("/notes/{id}", get(|| async { "hello" })),
		ServerConfig {
			export: None,
			resource: Resource {
				service: "notes".into(),
				environment: "local".into(),
				release: "dev".into(),
				instance: "one".into(),
			},
			limits: Limits::default(),
			read_key: key,
			public_origin: "http://localhost:3000".into(),
			browser_routes: vec!["/notes/{id}".into()],
		},
	)?;
	let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
	axum::serve(listener, app).await?;
	Ok(())
}
