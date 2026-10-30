//! Platform host registration generated from this runner's dependencies.
#[cfg(target_os = "macos")]
include!(concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../.leptoskit/macos/registrant.rs"
));
