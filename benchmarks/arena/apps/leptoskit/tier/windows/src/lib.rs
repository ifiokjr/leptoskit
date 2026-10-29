//! Platform host registration generated from this runner's dependencies.
#[cfg(target_os = "windows")]
include!(concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../.leptoskit/windows/registrant.rs"
));
