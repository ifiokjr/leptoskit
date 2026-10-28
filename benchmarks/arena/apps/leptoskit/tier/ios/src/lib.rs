//! Platform host registration generated from this runner's dependencies.
#[cfg(target_os = "ios")]
include!(concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../.leptoskit/ios/registrant.rs"
));
