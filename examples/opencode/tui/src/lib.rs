//! Platform host registration generated from this runner's dependencies.
#[cfg(not(target_arch = "wasm32"))]
include!(concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../.leptoskit/tui/registrant.rs"
));
