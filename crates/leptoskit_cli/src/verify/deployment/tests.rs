use super::*;

const NOW: u64 = 1_800_000_000;

fn key(seed: u8) -> SigningKey {
	SigningKey::from_bytes(&[seed; 32])
}

fn policy() -> serde_json::Value {
	serde_json::json!({
		"format_version": 1,
		"kind": "leptoskit-deployment-policy",
		"repository": "https://github.com/example/app",
		"deployment": "production",
		"origins": ["https://example.com", "https://api.example.com"],
		"sequence": 5,
		"issued_at": NOW - 60,
		"expires_at": NOW + 86_400,
		"releases": [{
			"commit": "a".repeat(40),
			"inventory_sha256": "b".repeat(64),
			"executable_sha256": "c".repeat(64),
		}],
	})
}

fn signed(policy: &serde_json::Value, key: &SigningKey) -> Vec<u8> {
	serde_json::to_vec(&sign(&serde_json::to_vec(policy).unwrap(), key).unwrap()).unwrap()
}

fn check(envelope: &[u8], origin: &str) -> Result<Verified> {
	verify(
		envelope,
		&[key(1).verifying_key()],
		origin,
		Some("https://github.com/example/app"),
		NOW,
	)
}

#[test]
fn a_trusted_signature_approves_its_releases_for_its_own_origins() {
	let envelope = signed(&policy(), &key(1));
	let verified = check(&envelope, "https://api.example.com").unwrap();
	assert_eq!(verified.policy.sequence, 5);
	assert_eq!(verified.policy.releases[0].inventory_sha256, "b".repeat(64));
	assert_eq!(
		verified.keyid,
		hex::encode(key(1).verifying_key().as_bytes())
	);
	assert_eq!(verified.sha256.len(), 64);
}

#[test]
fn a_copied_policy_does_not_vouch_for_another_origin() {
	let envelope = signed(&policy(), &key(1));
	let error = check(&envelope, "https://example.com.evil.test").unwrap_err();
	assert!(
		error
			.to_string()
			.contains("not one of this deployment's origins"),
		"{error}"
	);
}

#[test]
fn a_policy_naming_a_new_origin_needs_the_trusted_key() {
	let mut forged = policy();
	forged["origins"] = serde_json::json!(["https://evil.test"]);
	let error = check(&signed(&forged, &key(2)), "https://evil.test").unwrap_err();
	assert!(error.to_string().contains("key you trust"), "{error}");

	// The trusted signature does not survive a changed payload.
	let mut envelope: Envelope = serde_json::from_slice(&signed(&policy(), &key(1))).unwrap();
	envelope.payload = STANDARD.encode(serde_json::to_vec(&forged).unwrap());
	let error = check(&serde_json::to_vec(&envelope).unwrap(), "https://evil.test").unwrap_err();
	assert!(error.to_string().contains("key you trust"), "{error}");
}

#[test]
fn a_signature_over_the_bare_policy_is_not_a_policy_signature() {
	let payload = serde_json::to_vec(&policy()).unwrap();
	let mut envelope: Envelope = serde_json::from_slice(&signed(&policy(), &key(1))).unwrap();
	// The same key signing the same bytes for some other purpose, without
	// the DSSE type binding, must not be accepted.
	envelope.signatures[0].sig = STANDARD.encode(key(1).sign(&payload).to_bytes());
	assert!(
		check(
			&serde_json::to_vec(&envelope).unwrap(),
			"https://example.com"
		)
		.is_err()
	);
	envelope.payload_type = "application/vnd.in-toto+json".to_owned();
	assert!(
		check(
			&serde_json::to_vec(&envelope).unwrap(),
			"https://example.com"
		)
		.is_err()
	);
}

#[test]
fn expired_future_and_foreign_policies_are_refused() {
	let mut expired = policy();
	expired["expires_at"] = NOW.into();
	let error = check(&signed(&expired, &key(1)), "https://example.com").unwrap_err();
	assert!(error.to_string().contains("expired"), "{error}");

	let mut future = policy();
	future["issued_at"] = (NOW + 3_600).into();
	future["expires_at"] = (NOW + 7_200).into();
	let error = check(&signed(&future, &key(1)), "https://example.com").unwrap_err();
	assert!(error.to_string().contains("future"), "{error}");

	let mut fork = policy();
	fork["repository"] = "https://github.com/someone-else/app".into();
	let error = check(&signed(&fork, &key(1)), "https://example.com").unwrap_err();
	assert!(
		error
			.to_string()
			.contains("not https://github.com/example/app"),
		"{error}"
	);
}

#[test]
fn policies_must_be_complete_and_canonical_before_they_are_signed() {
	for (field, value) in [
		("origins", serde_json::json!([])),
		("origins", serde_json::json!(["http://example.com"])),
		("origins", serde_json::json!(["https://example.com/app"])),
		("origins", serde_json::json!(["https://Example.com:443"])),
		("releases", serde_json::json!([])),
		(
			"releases",
			serde_json::json!([{ "commit": "abc", "inventory_sha256": "b".repeat(64), "executable_sha256": "c".repeat(64) }]),
		),
		(
			"repository",
			serde_json::json!("https://github.com/example/app.git"),
		),
		(
			"repository",
			serde_json::json!("https://github.com/example/app.GIT"),
		),
		(
			"repository",
			serde_json::json!("https://github.com/example/app/"),
		),
		(
			"repository",
			serde_json::json!("https://gitlab.com/example/app"),
		),
		("deployment", serde_json::json!("prod uction")),
		("expires_at", serde_json::json!(NOW - 60)),
		(
			"kind",
			serde_json::json!("leptoskit-backend-bundle-inventory"),
		),
	] {
		let mut value_policy = policy();
		value_policy[field] = value;
		assert!(
			sign(&serde_json::to_vec(&value_policy).unwrap(), &key(1)).is_err(),
			"{field}: {}",
			value_policy[field]
		);
	}

	// A field this verifier does not understand is refused, not ignored.
	let mut extended = policy();
	extended["attested"] = true.into();
	assert!(sign(&serde_json::to_vec(&extended).unwrap(), &key(1)).is_err());
}

#[test]
fn origins_are_compared_in_canonical_form() {
	assert_eq!(
		origin("https://EXAMPLE.com:443/").unwrap(),
		"https://example.com"
	);
	assert_eq!(
		origin("https://example.com:8443").unwrap(),
		"https://example.com:8443"
	);
	for value in [
		"http://example.com",
		"http://127.0.0.1:8080",
		"https://user@example.com",
		"https://example.com/?q",
		"https://example.com/app",
	] {
		assert!(origin(value).is_err(), "{value}");
	}
}

#[test]
fn remembered_state_refuses_rollbacks_and_conflicting_policies() {
	let directory = tempfile::tempdir().unwrap();
	let state = directory.path().join("state.json");
	let at = |sequence: u64, releases_digest: char| {
		let mut value = policy();
		value["sequence"] = sequence.into();
		value["releases"][0]["executable_sha256"] = releases_digest.to_string().repeat(64).into();
		check(&signed(&value, &key(1)), "https://example.com").unwrap()
	};

	remember(&state, &at(5, 'c')).unwrap();
	remember(&state, &at(5, 'c')).unwrap();
	let error = remember(&state, &at(4, 'c')).unwrap_err();
	assert!(error.to_string().contains("rollback"), "{error}");
	let error = remember(&state, &at(5, 'd')).unwrap_err();
	assert!(
		error.to_string().contains("two different policies"),
		"{error}"
	);
	remember(&state, &at(6, 'd')).unwrap();
	assert!(remember(&state, &at(5, 'c')).is_err());

	let mut staging = policy();
	staging["deployment"] = "staging".into();
	let staging = check(&signed(&staging, &key(1)), "https://example.com").unwrap();
	assert!(remember(&state, &staging).is_err());
}

#[cfg(unix)]
#[test]
fn keys_are_private_and_never_overwritten() {
	use std::os::unix::fs::PermissionsExt as _;

	let directory = tempfile::tempdir().unwrap();
	let path = directory.path().join("deployment.key");
	keygen(&KeygenArgs { out: path.clone() }).unwrap();
	let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
	assert_eq!(mode, 0o600);
	let key = read_key(&path).unwrap();
	assert!(keygen(&KeygenArgs { out: path.clone() }).is_err());
	assert_eq!(read_key(&path).unwrap().to_bytes(), key.to_bytes());
}

#[test]
fn backend_binding_requires_v2_and_preserves_replay_protection() {
	let mut value = policy();
	value["releases"][0]["backend"] = serde_json::json!({"descriptor_sha256":"d".repeat(64),"engine_image_id":format!("sha256:{}","e".repeat(64))});
	assert!(sign(&serde_json::to_vec(&value).unwrap(), &key(1)).is_err());
	value["format_version"] = 2.into();
	let verified = check(&signed(&value, &key(1)), "https://example.com").unwrap();
	let dir = tempfile::tempdir().unwrap();
	let state = dir.path().join("state.json");
	remember(&state, &verified).unwrap();
	value["releases"][0]["backend"]["descriptor_sha256"] = serde_json::json!("f".repeat(64));
	let conflicting = check(&signed(&value, &key(1)), "https://example.com").unwrap();
	assert!(remember(&state, &conflicting).is_err());
	value["sequence"] = 4.into();
	let replay = check(&signed(&value, &key(1)), "https://example.com").unwrap();
	assert!(remember(&state, &replay).is_err());
}
