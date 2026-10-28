//! Interpreted guests use optimized profiles independently of their native
//! host.
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Optimize;

use crate::wasm;
use crate::wasm::CargoOptions;

/// Keeps guest optimization active when the surrounding host is a debug build.
pub(super) fn select(config: &LeptoskitConfig, mut options: CargoOptions) -> CargoOptions {
	options.release = true;

	match (&config.bundle.cargo_profile, config.bundle.optimize) {
		(Some(profile), _) => options.with_profile(profile, Vec::new()),
		(None, Optimize::Smallest) => {
			options.with_profile(wasm::SMALLEST_PROFILE, wasm::smallest_profile_config())
		}
		(None, Optimize::Fast) => options,
	}
}

#[cfg(test)]
mod tests {
	use std::path::Path;

	use super::*;

	fn arguments(config: &LeptoskitConfig, host_release: bool) -> Vec<String> {
		let options = select(
			config,
			CargoOptions {
				release: host_release,
				features: vec!["native".into()],
				..Default::default()
			},
		);
		let command = wasm::cargo_build_command(Path::new("."), "fixture", &options);
		command
			.get_args()
			.map(|argument| argument.to_str().unwrap().to_owned())
			.collect()
	}

	#[test]
	fn debug_host_still_builds_an_optimized_guest_with_its_native_feature() {
		let config = LeptoskitConfig::default();
		let baseline =
			wasm::cargo_build_command(Path::new("."), "fixture", &CargoOptions::default());
		assert!(!baseline.get_args().any(|argument| argument == "--release"));

		for host_release in [false, true] {
			let arguments = arguments(&config, host_release);
			assert!(arguments.iter().any(|argument| argument == "--release"));
			assert!(
				arguments
					.windows(2)
					.any(|pair| pair == ["--features", "native"])
			);
		}

		assert_eq!(
			super::super::profile_directory(&config, true, false),
			"debug"
		);
	}

	#[test]
	fn size_profile_and_explicit_profile_apply_to_debug_native_builds() {
		let mut config = LeptoskitConfig::default();
		config.bundle.optimize = Optimize::Smallest;

		for host_release in [false, true] {
			assert!(
				arguments(&config, host_release)
					.windows(2)
					.any(|pair| pair == ["--profile", wasm::SMALLEST_PROFILE])
			);
		}

		config.bundle.cargo_profile = Some("project-wasm".into());

		for host_release in [false, true] {
			assert!(
				arguments(&config, host_release)
					.windows(2)
					.any(|pair| pair == ["--profile", "project-wasm"])
			);
		}
	}
}
