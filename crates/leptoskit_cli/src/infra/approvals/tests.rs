use super::*;
const SHA256_HEX_LENGTH: usize = 64;
const COMMIT_HEX_LENGTH: usize = 40;
const NOW: u64 = 1_800_000_000;
fn signing(seed: u8) -> SigningKey {
	SigningKey::from_bytes(&[seed; 32])
}
fn fixture() -> (Evidence, Descriptor, Trust) {
	let value = serde_json::json!({
		"format_version": 1, "kind": "leptoskit-verified-backend-image",
		"inventory_sha256": "a".repeat(SHA256_HEX_LENGTH),
		"recipe": {"repository": "https://github.com/example/backend", "commit": "a".repeat(COMMIT_HEX_LENGTH),
			"builder_image": format!("builder@sha256:{}", "a".repeat(SHA256_HEX_LENGTH)),
			"target": "aarch64-unknown-linux-musl", "package": "backend", "binary": "backend",
			"features": [], "no_default_features": false, "rustflags": []},
		"executable_sha256": "b".repeat(SHA256_HEX_LENGTH), "executable_mode": 493,
		"migrations": &"a".repeat(SHA256_HEX_LENGTH),
		"runtime_image": format!("runtime@sha256:{}", "a".repeat(SHA256_HEX_LENGTH)),
		"runtime_config_sha256": "c".repeat(SHA256_HEX_LENGTH),
		"engine_image_id": format!("sha256:{}", "d".repeat(SHA256_HEX_LENGTH)),
		"image_config_sha256": "e".repeat(SHA256_HEX_LENGTH), "platform": "linux/arm64",
		"assets": "not_applicable", "services": "unsupported",
		"rpc_contract": {"state": "not_applicable", "declaration": "operator_declares_no_rpc"},
		"native_guest_compatibility": "not_applicable", "runtime_attestation": "absent"
	});
	let descriptor: Descriptor = serde_json::from_value(value).unwrap();
	let bytes = serde_json::to_string_pretty(&descriptor).unwrap();
	let policy = serde_json::json!({
"format_version":2,"kind":"leptoskit-deployment-policy","repository":descriptor.recipe.repository,
"deployment":"production","origins":["https://example.com"],"sequence":5,"issued_at":NOW-60,"expires_at":NOW+60,
"releases":[{"commit":descriptor.recipe.commit,"inventory_sha256":descriptor.inventory_sha256,"executable_sha256":descriptor.executable_sha256,
"backend":{"descriptor_sha256":hash(bytes.as_bytes()),"engine_image_id":descriptor.engine_image_id}}]});
	let envelope = deployment::sign(&serde_json::to_vec(&policy).unwrap(), &signing(3)).unwrap();
	let evidence = Evidence {
		format_version: 1,
		approvals: vec![
			sign(bytes.as_bytes(), &signing(1)).unwrap(),
			sign(bytes.as_bytes(), &signing(2)).unwrap(),
		],
		descriptor: bytes,
		deployment_policy: serde_json::to_string(&envelope).unwrap(),
	};
	let trust = Trust {
		configuration: Configuration {
			format_version: 1,
			repository: descriptor.recipe.repository.clone(),
			deployment: "production".into(),
			publisher: hex::encode(signing(4).verifying_key().as_bytes()),
			approvers: vec![],
			threshold: 2,
			deployment_keys: vec![],
		},
		approvers: vec![signing(1).verifying_key(), signing(2).verifying_key()],
		deployment_keys: vec![signing(3).verifying_key()],
	};
	(evidence, descriptor, trust)
}
fn check(
	evidence: &Evidence,
	descriptor: &Descriptor,
	trust: &Trust,
) -> Result<deployment::Verified> {
	verify(evidence, descriptor, trust, "https://example.com", NOW)
}
#[test]
fn independent_approvals_and_exact_policy_pass() {
	let (evidence, descriptor, trust) = fixture();
	check(&evidence, &descriptor, &trust).unwrap();
	evidence
		.check_pin(&hash(evidence.descriptor.as_bytes()))
		.unwrap();
	assert!(
		evidence
			.check_pin(&hash(format!("{} ", evidence.descriptor).as_bytes()))
			.is_err()
	);
}
#[test]
fn duplicates_and_publisher_only_do_not_count() {
	let (mut evidence, descriptor, trust) = fixture();
	evidence.approvals[1] = evidence.approvals[0].clone();
	assert!(check(&evidence, &descriptor, &trust).is_err());
	evidence.approvals = vec![sign(evidence.descriptor.as_bytes(), &signing(4)).unwrap()];
	assert!(check(&evidence, &descriptor, &trust).is_err());
}
#[test]
fn all_descriptor_fields_are_bound() {
	let (evidence, descriptor, trust) = fixture();
	for field in [
		"inventory_sha256",
		"executable_sha256",
		"migrations",
		"image_config_sha256",
		"runtime_config_sha256",
		"engine_image_id",
		"rpc_contract",
		"recipe",
	] {
		let mut value = serde_json::to_value(&descriptor).unwrap();

		match field {
			"recipe" => value[field]["commit"] = serde_json::json!("f".repeat(COMMIT_HEX_LENGTH)),
			"rpc_contract" => {
				value[field] = serde_json::json!({"state":"exported","sha256":"f".repeat(SHA256_HEX_LENGTH),
		"compatibility":"not_checked"});
			}
			"engine_image_id" => {
				value[field] =
					serde_json::json!(format!("sha256:{}", "f".repeat(SHA256_HEX_LENGTH)));
			}
			_ => value[field] = serde_json::json!("f".repeat(SHA256_HEX_LENGTH)),
		}

		let changed: Descriptor = serde_json::from_value(value).unwrap();
		assert!(check(&evidence, &changed, &trust).is_err(), "{field}");
		let mut changed_e = evidence.clone();
		changed_e.descriptor = serde_json::to_string(&changed).unwrap();
		assert!(check(&changed_e, &changed, &trust).is_err(), "{field}");
	}
	let mut whitespace = evidence.clone();
	whitespace.descriptor.push(' ');
	assert!(check(&whitespace, &descriptor, &trust).is_err());
}
// Re-sign each altered statement so these cases test domain and commitment
// rejection independently of ordinary signature tampering.
#[test]
fn wrong_domain_kind_and_conflicting_commit_fail() {
	let (evidence, descriptor, trust) = fixture();

	for change in ["domain", "kind", "commit"] {
		let mut altered = evidence.clone();
		let approval = &mut altered.approvals[0];
		let mut statement: Statement = serde_json::from_str(&approval.statement).unwrap();

		if change == "kind" {
			statement.kind = "leptoskit-source-approval".into();
		}

		if change == "commit" {
			statement.commit = "f".repeat(COMMIT_HEX_LENGTH);
		}

		approval.statement = serde_json::to_string(&statement).unwrap();
		let msg = if change == "domain" {
			approval.statement.as_bytes().to_vec()
		} else {
			message(&approval.statement)
		};

		approval.signature = hex::encode(signing(1).sign(&msg).to_bytes());
		assert!(check(&altered, &descriptor, &trust).is_err());
	}
}
#[test]
fn expired_wrong_origin_and_legacy_policies_fail() {
	let (mut evidence, descriptor, trust) = fixture();
	assert!(
		verify(
			&evidence,
			&descriptor,
			&trust,
			"https://elsewhere.example",
			NOW
		)
		.is_err()
	);
	assert!(
		verify(
			&evidence,
			&descriptor,
			&trust,
			"https://example.com",
			NOW + 60
		)
		.is_err()
	);
	let legacy = serde_json::json!({"format_version":1,
		"kind":"leptoskit-deployment-policy",
		"repository":descriptor.recipe.repository,
		"deployment":"production",
		"origins":["https://example.com"],
		"sequence":6,
		"issued_at":NOW-60,
		"expires_at":NOW+60,
		"releases":[{"commit":descriptor.recipe.commit,"inventory_sha256":descriptor.inventory_sha256,"executable_sha256":descriptor.executable_sha256}]});
	evidence.deployment_policy = serde_json::to_string(
		&deployment::sign(&serde_json::to_vec(&legacy).unwrap(), &signing(3)).unwrap(),
	)
	.unwrap();
	assert!(check(&evidence, &descriptor, &trust).is_err());
}
#[test]
fn configured_policy_never_falls_back_to_pin_mode() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("backend-trust.json"), "{}").unwrap();
	assert!(authorize(dir.path(), "example.com", None, None).is_err());
	let (evidence, descriptor, _) = fixture();
	std::fs::remove_file(dir.path().join("backend-trust.json")).unwrap();
	assert!(
		authorize(
			dir.path(),
			"example.com",
			Some(&descriptor),
			Some(&evidence)
		)
		.is_err()
	);
}
#[test]
fn key_aliases_and_publisher_overlap_rejected() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("trust.json");
	let public = hex::encode(signing(1).verifying_key().as_bytes());
	for (publisher, approvers) in [
		(
			hex::encode(signing(4).verifying_key().as_bytes()),
			vec![public.clone(), public.to_uppercase()],
		),
		(public.clone(), vec![public.clone()]),
	] {
		std::fs::write(
			&path,
			serde_json::to_vec(&serde_json::json!({"format_version":1,
		"repository":"https://github.com/example/backend",
		"deployment":"production",
		"publisher":publisher,
		"approvers":approvers,
		"threshold":1,
		"deployment_keys":[public]}))
			.unwrap(),
		)
		.unwrap();
		assert!(Trust::read(&path).is_err());
	}
}
