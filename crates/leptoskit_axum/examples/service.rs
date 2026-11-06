//! The smallest service: `main!` links the operations library (here this
//! crate stands in for it), reads the environment, and serves the router.
//!
//! Run it as `leptoskit dev` would:
//!
//! ```sh
//! LEPTOSKIT_SERVICE_NAME=hello LEPTOSKIT_SERVICE_ADDR=127.0.0.1:3100 \
//! LEPTOSKIT_SERVICE_TOKEN=secret cargo run -p leptoskit_axum --example service
//! ```

use leptoskit_axum::axum::Router;
use leptoskit_axum::axum::routing::get;
use leptoskit_axum::service::ServiceEnv;

leptoskit_axum::service::main!(leptoskit_axum, run);

async fn run(env: ServiceEnv) -> Result<Router, std::convert::Infallible> {
	let public = Router::new().route("/hello", get(|| async { "hello" }));
	let private = Router::new()
		.route("/whoami", get(|| async { "the app" }))
		.route_layer(env.require_token());
	Ok(Router::new().nest(&env.public_prefix(), public.merge(private)))
}
