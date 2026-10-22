//! Platform identities shared by native hosts and their update publisher.

use std::path::Path;

use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;

/// Universal projects own a Cargo runner in each native platform directory.
pub(crate) fn universal(root: &Path) -> bool {
	Platform::APPLICATION.into_iter().any(|platform| {
		platform.is_native() && root.join(platform.to_string()).join("Cargo.toml").is_file()
	})
}

/// Each destination needs its own latest-release pointer and build sequence.
pub(crate) fn channel(root: &Path, base: &str, platform: Platform) -> String {
	if platform.is_native() && universal(root) {
		return format!("{base}-{platform}");
	}

	base.to_owned()
}

/// Platform features produce incompatible guests even with the same host ABI.
pub(crate) fn shell(root: &Path, config: &LeptoskitConfig, platform: Platform) -> String {
	if platform.is_native() && universal(root) {
		return format!("{}/{platform}", config.native.update_contract);
	}

	config.native.update_contract.clone()
}
