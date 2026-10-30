//! Platform host registration generated from this runner's dependencies.
#[cfg(target_os = "android")]
include!(concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../.leptoskit/android/registrant.rs"
));
