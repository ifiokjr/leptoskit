use super::*;

#[test]
fn real_platform_generators_emit_readable_constructor_signatures() {
	let temporary = tempfile::tempdir().unwrap();
	let temporary_path = temporary.path().canonicalize().unwrap();
	let destination = std::env::var_os("LEPTOSKIT_QUALITY_OUTPUT");
	let output = destination.map(std::path::PathBuf::from);
	let base = output.as_deref().unwrap_or(&temporary_path);

	for name in ["demo", "solana_journey"] {
		let root = base.join(name);
		let mut registry = registry(&root);
		let mut config = leptoskit_config::LeptoskitConfig::default();
		config.app.name = name.into();

		if name == "solana_journey" {
			let mut plugin: super::super::registry::Plugin =
				serde_json::from_value(serde_json::json!({
					"name": "solana-journey",
					"rust-service-factory": "from_app_dirs"
				}))
				.unwrap();
			plugin.crate_name = "solana_kit_app_journey_service".into();
			registry.plugins.push(plugin);
		}

		write(
			&root.join("hosts/macos/App.swift"),
			include_str!("../templates/MacApp.swift"),
		)
		.unwrap();
		write(
			&root.join("hosts/ios/App.swift"),
			include_str!("../templates/App.swift"),
		)
		.unwrap();
		write(&root.join("hosts/android/app/src/main/AndroidManifest.xml"),
			"<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\"><application/></manifest>").unwrap();

		for target in [Target::Macos, Target::Ios, Target::Android] {
			generate(&root, &config, &registry, target, false).unwrap();
			let generated = super::super::generated_dir(&root, target);
			let source = std::fs::read_to_string(generated.join("registrant.rs")).unwrap();

			assert!(source.lines().all(|line| line.chars().count() <= 100));
			assert!(source.contains("create_with_context("));
			assert!(source.contains("Ok(())"));
		}
	}
}

fn registry(root: &Path) -> Registry {
	Registry {
		package: "solana_journey_macos".into(),
		plugins: vec![],
		support: vec![],
		bridge: Path::new(env!("CARGO_MANIFEST_DIR")).join("../leptoskit_bridge_ffi"),
		target: root.join("target"),
		host_features: vec![],
		devtools_feature: None,
		missing: Vec::new(),
	}
}
