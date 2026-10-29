//! Platform host registration generated from this runner's dependencies.
#[cfg(target_os = "linux")]
include!(concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../.leptoskit/linux/registrant.rs"
));
