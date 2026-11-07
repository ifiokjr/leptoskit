//! Exercise the full command boundary without cloud accounts or SSH hosts.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::Command;
use std::process::Output;

const CONFIG: &str = r#"
[app]
name = "notes"
render = "ssr"
[serve]
probes = true
[db]
dialect = "postgres"
[infra]
name = "notes-production"
domain = "app.example.com"
location = "hel1"
server_type = "cx23"
ssh_key_ids = [123]
ssh_cidrs = ["192.0.2.1/32"]
account_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
zone_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
updates_domain = "updates.example.com"
updates_bucket = "notes-releases"
backup_bucket = "notes-backups"
image = "ghcr.io/team/notes"
registry = "ghcr.io"
registry_username = "team"
"#;

fn project() -> tempfile::TempDir {
	let root = tempfile::tempdir().unwrap();
	fs::create_dir_all(root.path().join("src")).unwrap();
	fs::create_dir_all(root.path().join("migrations")).unwrap();
	fs::create_dir_all(root.path().join("bin")).unwrap();
	fs::write(
		root.path().join("Cargo.toml"),
		"[package]\nname='notes'\nversion='0.1.0'\n",
	)
	.unwrap();
	fs::write(root.path().join("leptoskit.toml"), CONFIG).unwrap();
	fs::write(
		root.path().join("src/main.rs"),
		"fn main() { /* readyz */ }",
	)
	.unwrap();
	fs::write(root.path().join("migrations/registry.txt"), "").unwrap();
	tool(
		root.path(),
		"tofu",
		r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> tofu-calls
case "$1" in
init) if [ -n "${MOCK_TOFU_INIT_EXIT:-}" ]; then exit 1; fi; printf lock > .terraform.lock.hcl ;;
plan) printf plan > leptoskit.plan ;;
show)
  if [ -n "${MOCK_SHOW_EXIT:-}" ]; then exit 1; fi
  if [ -n "${MOCK_PLAN_JSON:-}" ]; then printf '%s' "$MOCK_PLAN_JSON";
  else printf '%s' '{"format_version":"1.2","resource_changes":[]}'; fi ;;
output)
  if [ -n "${MOCK_OUTPUT_FAIL:-}" ]; then exit 1; fi
  if [ "$#" -eq 2 ]; then
    if [ -n "${MOCK_OUTPUT_JSON:-}" ]; then printf '%s' "$MOCK_OUTPUT_JSON";
    else printf '{"server_ip":{"value":"192.0.2.10"}}'; fi
    exit 0
  fi
  if [ -n "${MOCK_OUTPUT_RAW_FAIL:-}" ]; then exit 1; fi
  case "$3" in
  server_ip) if [ -n "${MOCK_SERVER_IP:-}" ]; then printf '%s' "$MOCK_SERVER_IP"; else printf '192.0.2.10'; fi ;;
  server_ips) if [ -n "${MOCK_HOSTS:-}" ]; then printf '%s' "$MOCK_HOSTS"; else printf '["192.0.2.10"]'; fi ;;
  private_ip) if [ -n "${MOCK_PRIVATE_IP:-}" ]; then printf '%s' "$MOCK_PRIVATE_IP"; else printf '10.42.0.10'; fi ;;
  backup_endpoint) if [ -n "${MOCK_BACKUP_ENDPOINT:-}" ]; then printf '%s' "$MOCK_BACKUP_ENDPOINT";
    else printf 'https://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.r2.cloudflarestorage.com'; fi ;;
  app_url) printf 'https://app.example.com' ;;
  updates_url) printf 'https://updates.example.com' ;;
  updates_bucket) printf 'notes-releases' ;;
  backup_bucket) printf 'notes-backups' ;;
  s3_endpoint) printf 'https://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.r2.cloudflarestorage.com' ;;
  tunnel_token) printf 'fixture-private-tunnel-token' ;;
  esac ;;
esac
"#,
	);
	tool(
		root.path(),
		"kamal",
		r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> kamal-calls
if [ "$1" = app ] && [ "$2" = boot ]; then
  printf '%s\n' "${LEPTOSKIT_RELEASE:-missing}" >> boot-versions
  if [ -n "${MOCK_KAMAL_FAIL_VERSION:-}" ] && [ "${LEPTOSKIT_RELEASE:-}" = "$MOCK_KAMAL_FAIL_VERSION" ]; then exit 1; fi
fi
exit "${MOCK_KAMAL_EXIT:-0}"
"#,
	);
	tool(
		root.path(),
		"docker",
		r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> docker-calls
if [ "$1" = manifest ]; then
  if [ "${MOCK_REGISTRY_EXISTS:-0}" = 1 ]; then exit 0; fi
  printf 'manifest unknown' >&2; exit 1
fi
if [ "$1" = image ]; then printf '%s' "$MOCK_SCHEMA_LABEL"; fi
if [ "$1" = login ]; then cat >/dev/null; fi
if [ "$1" = build ]; then exit "${MOCK_BUILD_EXIT:-0}"; fi
exit 0
"#,
	);
	tool(
		root.path(),
		"ssh",
		r#"#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys
script = sys.argv[-1]
target = sys.argv[-2]
root = pathlib.Path(os.environ["MOCK_REMOTE_ROOT"])
(root / "etc/systemd/system").mkdir(parents=True, exist_ok=True)
with open(root / "ssh-calls", "a") as log:
    log.write(script + "\n")
if os.environ.get("MOCK_SSH_EXIT"):
    sys.exit(1)
if "pg_dump" in script:
    if os.environ.get("MOCK_PG_DUMP_EXIT"):
        sys.exit(1)
    print(os.environ.get("MOCK_PG_DUMP", "PGDMPfixture"), end="")
elif "io.leptoskit.migrations" in script:
    print(os.environ["MOCK_SCHEMA_LABEL"])
elif "json .RepoDigests" in script:
    # A running container's digest (`docker inspect` first) can diverge from
    # the pulled image's (`docker image inspect`), as it does mid-rollout.
    if script.startswith("docker image inspect"):
        digest = os.environ.get("MOCK_SECONDARY_DIGEST") if target.endswith("192.0.2.11") else None
    else:
        digest = os.environ.get("MOCK_CONTAINER_SECONDARY_DIGEST") if target.endswith("192.0.2.11") else os.environ.get("MOCK_CONTAINER_DIGEST")
    print(json.dumps(["ghcr.io/team/notes@sha256:" + (digest or os.environ.get("MOCK_DIGEST", "a" * 64))]))
elif script.startswith("docker image inspect") and "--format" not in script:
    changed = os.environ.get("MOCK_IMAGE_MISMATCH") == "all" or (os.environ.get("MOCK_IMAGE_MISMATCH") == "postboot" and "@sha256:" in script)
    print(json.dumps([{"Id": "sha256:" + ("b" if changed else "a") * 64, "Config": {}, "Os": "linux", "Architecture": "arm64"}]))
elif script.startswith("docker exec") and "_leptoskit_migrations" in script:
    if os.environ.get("MOCK_HISTORY_FAIL"):
        sys.exit(1)
    count_path = root / "history-count"
    count = int(count_path.read_text()) if count_path.exists() else 0
    count_path.write_text(str(count + 1))
    after = os.environ.get("MOCK_HISTORY_AFTER")
    print(after if after and count >= 1 else os.environ.get("MOCK_HISTORY", "[]"))
elif script.startswith("python3 "):
    print(json.dumps({"healthy": True, "verified": True, "snapshot": "a" * 64}))
elif "docker inspect --format" in script and ".State.Status" in script:
    status = os.environ.get("MOCK_ACCESSORY_STATUS")
    if status is None:
        sys.exit(1)
    print(status)
elif "pg_stat_activity" in script:
    print(os.environ.get("MOCK_DB_CLIENTS", "0"), end="")
elif ".State.Status" in script:
    sys.exit(1)
elif "install -d" in script:
    pass
elif script.startswith("systemd-analyze") or script.startswith("systemctl") or script.startswith("cloud-init") or "docker ps -q" in script or "pg_isready" in script:
    pass
else:
    script = script.replace("/var/lib/leptoskit", str(root / "state")).replace("/etc/systemd/system", str(root / "etc/systemd/system"))
    sys.exit(subprocess.run(["/bin/sh", "-c", script]).returncode)
"#,
	);
	root
}

fn tool(root: &Path, name: &str, source: &str) {
	let path = root.join("bin").join(name);
	fs::write(&path, source).unwrap();
	fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn cli(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
	let paths = std::iter::once(root.join("bin"))
		.chain(std::env::split_paths(
			&std::env::var_os("PATH").unwrap_or_default(),
		))
		.collect::<Vec<_>>();
	Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.current_dir(root)
		.args(args)
		.env("PATH", std::env::join_paths(paths).unwrap())
		.env("MOCK_REMOTE_ROOT", root.join("remote"))
		.env("LEPTOSKIT_BACKUP_PASSWORD", "fixture-backup-encryption")
		.env("LEPTOSKIT_BACKUP_ACCESS_KEY_ID", "fixture-backup-key")
		.env(
			"LEPTOSKIT_BACKUP_SECRET_ACCESS_KEY",
			"fixture-backup-secret",
		)
		.env("MOCK_SCHEMA_LABEL", leptoskit_db::checksum("[]"))
		.env("NATS_TOKEN", "fixture-relay-token")
		.env("VALKEY_PASSWORD", "fixture-cache-password")
		.env("POSTGRES_PASSWORD", "fixture-password")
		.env("DATABASE_PASSWORD", "fixture-app-password")
		.env("KAMAL_REGISTRY_PASSWORD", "fixture-registry-password")
		// Provider price APIs are never called: no token leaks in from the
		// machine running the tests, and a test that needs one sets it below.
		.env_remove("HCLOUD_TOKEN")
		.env_remove("DIGITALOCEAN_TOKEN")
		.env_remove("AWS_ACCESS_KEY_ID")
		.env_remove("AWS_SECRET_ACCESS_KEY")
		.env_remove("LEPTOSKIT_ANALYTICS_READ_KEY")
		.envs(env.iter().copied())
		.output()
		.unwrap()
}

fn succeeds(output: &Output) {
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
}

fn fails_with(output: &Output, expected: &str) {
	assert!(
		!output.status.success(),
		"the command unexpectedly succeeded"
	);
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(
		stderr.contains(expected),
		"missing {expected:?} in:\n{stderr}"
	);
}

/// A fresh project whose `leptoskit.toml` is `config`, keeping the fixture
/// tools.
fn configured(config: &str) -> tempfile::TempDir {
	let root = project();
	fs::write(root.path().join("leptoskit.toml"), config).unwrap();
	root
}

/// Adds a migration after a project has already generated its configuration.
fn add_migration(root: &Path, name: &str) -> String {
	let sql = "SELECT 1;";
	fs::create_dir_all(root.join("migrations").join(name)).unwrap();
	fs::write(
		root.join("migrations").join(name).join("migration.sql"),
		sql,
	)
	.unwrap();
	let mut registry = fs::read_to_string(root.join("migrations/registry.txt")).unwrap();
	registry.push_str(name);
	registry.push('\n');
	fs::write(root.join("migrations/registry.txt"), registry).unwrap();
	leptoskit_db::checksum(sql)
}

fn operation(root: &Path) -> serde_json::Value {
	serde_json::from_slice(
		&fs::read(root.join("remote/state/notes-production/control/operation.json")).unwrap(),
	)
	.unwrap()
}

fn release_count(root: &Path) -> usize {
	serde_json::from_slice::<Vec<serde_json::Value>>(
		&fs::read(root.join("remote/state/notes-production/control/releases.json")).unwrap(),
	)
	.unwrap()
	.len()
}

struct ConsoleProcess<'a>(&'a mut std::process::Child);
impl Drop for ConsoleProcess<'_> {
	fn drop(&mut self) {
		let _ = self.0.kill();
		let _ = self.0.wait();
	}
}

fn source_build_tools(root: &Path) {
	tool(
		root,
		"git",
		r#"#!/bin/sh
set -eu
if [ "$1" = -C ]; then
  cd "$2"
  shift 2
fi
if [ "$1" = cat-file ]; then exit 0; fi
if [ "$1" = worktree ] && [ "$2" = add ]; then
  mkdir -p "$4"
  cp Cargo.toml leptoskit.toml "$4/"
  cp -R src migrations "$4/"
  exit 0
fi
if [ "$1" = worktree ] && [ "$2" = remove ]; then
  python3 -c 'import shutil,sys; shutil.rmtree(sys.argv[1])' "$4"
  exit 0
fi
exit 1
"#,
	);
	tool(
		root,
		"docker",
		r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$MOCK_REMOTE_ROOT/source-build-calls"
if [ "$1" = manifest ]; then printf 'manifest unknown' >&2; exit 1; fi
if [ "$1" = image ]; then printf '%s' "$MOCK_SCHEMA_LABEL"; fi
if [ "$1" = login ]; then cat >/dev/null; fi
exit 0
"#,
	);
}

#[test]
fn the_management_service_runs_existing_releases_after_the_request_client_disconnects() {
	use std::net::TcpListener;
	use std::time::Duration;
	use std::time::Instant;
	let root = project();
	let data = root.path().join("console-data");
	source_build_tools(root.path());
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	let address = listener.local_addr().unwrap();
	drop(listener);
	let url = format!("http://{address}");
	let owner = "test-service-owner-key-01234567890123456789";
	let paths = std::iter::once(root.path().join("bin"))
		.chain(std::env::split_paths(&std::env::var_os("PATH").unwrap()))
		.collect::<Vec<_>>();
	let mut child = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.current_dir(root.path())
		.args(["console", "serve", "--project"])
		.arg(root.path())
		.arg("--data")
		.arg(&data)
		.arg("--bind")
		.arg(address.to_string())
		.arg("--url")
		.arg(&url)
		.env("PATH", std::env::join_paths(paths).unwrap())
		.env("LEPTOSKIT_CONTROL_KEY", owner)
		.env("MOCK_REMOTE_ROOT", root.path().join("remote"))
		.env("POSTGRES_PASSWORD", "fixture-admin-password")
		.env("DATABASE_PASSWORD", "fixture-app-password")
		.env("KAMAL_REGISTRY_PASSWORD", "fixture-registry-password")
		.env("LEPTOSKIT_BACKUP_PASSWORD", "fixture-backup-encryption")
		.env("LEPTOSKIT_BACKUP_ACCESS_KEY_ID", "fixture-backup-key")
		.env(
			"LEPTOSKIT_BACKUP_SECRET_ACCESS_KEY",
			"fixture-backup-secret",
		)
		.env("MOCK_SCHEMA_LABEL", leptoskit_db::checksum("[]"))
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::null())
		.spawn()
		.unwrap();

	let _stop = ConsoleProcess(&mut child);
	let agent: ureq::Agent = ureq::Agent::config_builder()
		.timeout_global(Some(Duration::from_secs(5)))
		.max_redirects(0)
		.build()
		.into();
	let deadline = Instant::now() + Duration::from_secs(20);
	while agent.get(format!("{url}/healthz")).call().is_err() {
		assert!(
			Instant::now() < deadline,
			"management service did not start"
		);
		std::thread::sleep(Duration::from_millis(25));
	}
	let post = |revision: u64, key: &str, action: serde_json::Value| {
		let mut response = agent
			.post(format!("{url}/api/v1/projects/notes-production/operations"))
			.header("Authorization", format!("Bearer {owner}"))
			.send_json(
				serde_json::json!({"expected_revision":revision,"idempotency_key":key,"action":action}),
			)
			.unwrap();
		serde_json::from_str::<serde_json::Value>(&response.body_mut().read_to_string().unwrap())
			.unwrap()
	};
	let follow = |id: &str| {
		let deadline = Instant::now() + Duration::from_secs(20);
		loop {
			let mut response = agent
				.get(format!("{url}/api/v1/operations/{id}"))
				.header("Authorization", format!("Bearer {owner}"))
				.call()
				.unwrap();
			let operation: serde_json::Value =
				serde_json::from_str(&response.body_mut().read_to_string().unwrap()).unwrap();
			if operation["phase"] == "succeeded" {
				break operation;
			}
			assert!(
				operation["phase"] != "failed" && operation["phase"] != "interrupted",
				"{operation}"
			);
			assert!(
				Instant::now() < deadline,
				"operation timed out: {operation}"
			);
			std::thread::sleep(Duration::from_millis(25));
		}
	};
	let plan = post(0, "detached-plan", serde_json::json!({"kind":"plan"}));
	let id = plan["id"].as_str().unwrap();
	follow(id); // No original request connection remains open.
	let apply = post(
		1,
		"apply-reviewed-plan",
		serde_json::json!({"kind":"apply","plan_id":id}),
	);
	follow(apply["id"].as_str().unwrap());
	let deploy = post(
		2,
		"deploy-prebuilt-image",
		serde_json::json!({"kind":"deploy","version":"v1","setup":true}),
	);
	let operation = follow(deploy["id"].as_str().unwrap());
	assert!(!operation.to_string().contains(owner));
	let calls = fs::read_to_string(root.path().join("infra/notes-production/kamal-calls")).unwrap();
	assert!(calls.contains("app boot --version v1"));
	let connect = cli(
		root.path(),
		&["console", "connect", "--url", &url],
		&[
			("LEPTOSKIT_CONTROL_KEY", owner),
			(
				"XDG_CONFIG_HOME",
				root.path().join("private-config").to_str().unwrap(),
			),
		],
	);
	succeeds(&connect);
	let status = cli(
		root.path(),
		&["console", "status"],
		&[(
			"XDG_CONFIG_HOME",
			root.path().join("private-config").to_str().unwrap(),
		)],
	);
	succeeds(&status);
	assert!(!String::from_utf8(status.stdout).unwrap().contains(owner));
	let version = "0123456789012345678901234567890123456789";
	let source = post(
		3,
		"build-selected-source",
		serde_json::json!({"kind":"deploy","version":version}),
	);
	let result = follow(source["id"].as_str().unwrap());
	assert!(result["events"].as_array().unwrap().iter().any(|event| {
		event["message"]
			.as_str()
			.is_some_and(|message| message.contains("Building"))
	}));
	let builds = fs::read_to_string(root.path().join("remote/source-build-calls")).unwrap();
	assert!(builds.contains("build") && builds.contains(version) && builds.contains("push"));
	assert!(
		!data
			.join("workspaces")
			.join(source["id"].as_str().unwrap())
			.exists()
	);
}

#[test]
fn analytics_plugin_binds_release_identity_through_deploy_and_rollback() {
	let root = project();
	fs::write(
		root.path().join("leptoskit.toml"),
		format!("{CONFIG}\n[infra.plugins.analytics]\nbrowser_routes=['/notes/{{id}}']\n"),
	)
	.unwrap();
	let catalog = cli(root.path(), &["infra", "plugins"], &[]);
	succeeds(&catalog);
	let manifest: serde_json::Value = serde_json::from_slice(&catalog.stdout).unwrap();
	assert_eq!(manifest["plugins"][0]["enabled"], true);
	assert!(!root.path().join("tofu-calls").exists());
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	let key = "fixture-analytics-read-key-0123456789";
	for args in [
		vec!["deploy", "--setup", "--version", "v1"],
		vec!["deploy", "--version", "v2"],
		vec!["deploy", "--rollback", "v1"],
	] {
		succeeds(&cli(
			root.path(),
			&args,
			&[("LEPTOSKIT_ANALYTICS_READ_KEY", key)],
		));
	}
	assert_eq!(
		fs::read_to_string(root.path().join("infra/notes-production/boot-versions")).unwrap(),
		"v1\nv2\nv1\n"
	);
	let generated =
		fs::read_to_string(root.path().join("infra/notes-production/release.yml")).unwrap();
	assert!(generated.contains("LEPTOSKIT_ANALYTICS_READ_KEY"));
	assert!(!generated.contains(key));
	assert!(!String::from_utf8(catalog.stdout).unwrap().contains(key));
}

#[test]
fn saved_plan_is_consumed_and_public_output_omits_the_tunnel_token() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	assert!(root.path().join("Dockerfile").is_file());
	succeeds(&cli(root.path(), &["tofu", "plan"], &[]));
	succeeds(&cli(root.path(), &["infra", "apply"], &[]));
	assert!(!cli(root.path(), &["infra", "apply"], &[]).status.success());
	succeeds(&cli(root.path(), &["infra", "plan"], &[]));
	fs::write(
		root.path().join("infra/notes-production/leptoskit.plan"),
		"tampered",
	)
	.unwrap();
	assert!(!cli(root.path(), &["infra", "apply"], &[]).status.success());
	let output = cli(root.path(), &["infra", "output"], &[]);
	succeeds(&output);
	let text = String::from_utf8(output.stdout).unwrap();
	assert!(text.contains("192.0.2.10"));
	assert!(!text.contains("tunnel_token") && !text.contains("fixture-private"));
}

#[test]
fn releases_record_success_and_check_the_actual_database_before_rollback() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	let failed = cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[("MOCK_KAMAL_EXIT", "1")],
	);
	assert!(!failed.status.success());
	assert!(
		!root
			.path()
			.join("infra/notes-production/releases.json")
			.exists()
	);
	succeeds(&cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	succeeds(&cli(root.path(), &["deploy", "--version", "v2"], &[]));
	let changed = cli(
		root.path(),
		&["deploy", "--rollback", "v1"],
		&[("MOCK_HISTORY", "[[\"unknown\",\"checksum\"]]")],
	);
	assert!(!changed.status.success());
	succeeds(&cli(root.path(), &["deploy", "--rollback", "v1"], &[]));
	succeeds(&cli(root.path(), &["infra", "stop"], &[]));
	let calls = fs::read_to_string(root.path().join("infra/notes-production/kamal-calls")).unwrap();
	assert!(calls.contains("app boot --version v2 -c release.yml"));
	assert!(calls.contains("app boot --version v1 -c release.yml"));
	assert!(calls.contains("app boot --version v1 -c release.yml"));
	assert!(!calls.contains("fixture-password") && !calls.contains("fixture-private"));
	fs::create_dir(root.path().join("migrations/new_schema")).unwrap();
	fs::write(
		root.path().join("migrations/new_schema/migration.sql"),
		"SELECT 1;",
	)
	.unwrap();
	fs::write(root.path().join("migrations/registry.txt"), "new_schema\n").unwrap();
	assert!(
		!cli(root.path(), &["deploy", "--version", "v3"], &[])
			.status
			.success()
	);
}

#[test]
fn backups_are_private_and_never_overwrite_existing_files() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	succeeds(&cli(
		root.path(),
		&["infra", "backup", "--out", "backup.dump"],
		&[],
	));
	assert_eq!(
		fs::read(root.path().join("backup.dump")).unwrap(),
		b"PGDMPfixture"
	);
	let permissions = fs::metadata(root.path().join("backup.dump"))
		.unwrap()
		.permissions()
		.mode();
	assert_eq!(permissions & 0o077, 0);
	assert!(
		!cli(
			root.path(),
			&["infra", "backup", "--out", "backup.dump"],
			&[]
		)
		.status
		.success()
	);
}

#[test]
fn shared_history_and_owner_locks_survive_changing_operator_workspaces() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	succeeds(&cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	fs::remove_file(root.path().join("infra/notes-production/releases.json")).unwrap();
	let history = cli(root.path(), &["infra", "releases"], &[]);
	succeeds(&history);
	assert!(String::from_utf8_lossy(&history.stdout).contains("sha256:"));
	succeeds(&cli(root.path(), &["deploy", "--version", "v2"], &[]));
	let owner = "163e4567-e89b-42d3-a456-426614174000";
	let path = root
		.path()
		.join("remote/state/notes-production/control/operation.lock");
	fs::write(&path, owner).unwrap();
	assert!(
		!cli(root.path(), &["deploy", "--version", "v3"], &[])
			.status
			.success()
	);
	assert!(
		!cli(root.path(), &["infra", "unlock", "--owner", owner], &[])
			.status
			.success()
	);
	assert!(
		!cli(
			root.path(),
			&[
				"infra",
				"unlock",
				"--owner",
				"263e4567-e89b-42d3-a456-426614174000",
				"--yes"
			],
			&[]
		)
		.status
		.success()
	);
	assert_eq!(fs::read_to_string(&path).unwrap(), owner);
	succeeds(&cli(
		root.path(),
		&["infra", "unlock", "--owner", owner, "--yes"],
		&[],
	));
	assert!(!path.exists());
	succeeds(&cli(root.path(), &["infra", "status"], &[]));
}

#[test]
fn encrypted_backup_and_restore_commands_require_explicit_data_replacement() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	for args in [
		vec!["infra", "backups", "install"],
		vec!["infra", "backups", "run"],
		vec!["infra", "backups", "verify"],
		vec!["infra", "backups", "list"],
		vec!["infra", "backups", "status"],
	] {
		succeeds(&cli(root.path(), &args, &[]));
	}
	assert!(
		!cli(root.path(), &["infra", "restore"], &[])
			.status
			.success()
	);
	assert!(
		!cli(
			root.path(),
			&["deploy", "--rollback", "v1", "--rollback-schema"],
			&[]
		)
		.status
		.success()
	);
	succeeds(&cli(root.path(), &["infra", "restore", "--yes"], &[]));
	assert!(
		!cli(
			root.path(),
			&[
				"infra",
				"backups",
				"verify",
				"--snapshot",
				"../../etc/passwd"
			],
			&[]
		)
		.status
		.success()
	);
}

#[test]
fn rollback_refuses_a_changed_registry_tag_before_booting_it() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	succeeds(&cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	let failed = cli(
		root.path(),
		&["deploy", "--rollback", "v1"],
		&[(
			"MOCK_DIGEST",
			"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
		)],
	);
	assert!(!failed.status.success());
	let calls = fs::read_to_string(root.path().join("infra/notes-production/kamal-calls")).unwrap();
	assert!(calls.contains("build pull --version v1"));
	assert_eq!(calls.matches("app boot").count(), 1);
	let history = cli(root.path(), &["infra", "releases"], &[]);
	succeeds(&history);
	assert_eq!(
		serde_json::from_slice::<Vec<serde_json::Value>>(&history.stdout)
			.unwrap()
			.len(),
		1
	);
}

#[test]
fn ci_requires_shared_state_and_generates_a_reviewable_pipeline() {
	let root = project();
	assert!(!cli(root.path(), &["infra", "ci"], &[]).status.success());
	fs::write(
		root.path().join("leptoskit.toml"),
		format!(
			"{CONFIG}{}",
			r"
[infra.state]
bucket='independent-state'
key='production.tfstate'
region='eu-west-1'
"
		),
	)
	.unwrap();
	succeeds(&cli(root.path(), &["infra", "ci"], &[]));
	let path = root.path().join(".github/workflows/leptoskit-deploy.yml");
	let text = fs::read_to_string(&path).unwrap();
	let workflow: serde_json::Value = serde_json::from_str(&text).unwrap();
	assert_eq!(workflow["concurrency"]["cancel-in-progress"], false);
	assert_eq!(
		workflow["jobs"]["deploy"]["environment"],
		"notes-production"
	);
	assert!(text.contains("lk deploy --version"));
	assert!(!text.contains("fixture-password"));
	assert!(!text.contains("tofu apply"));
	fs::write(&path, "edited").unwrap();
	assert!(!cli(root.path(), &["infra", "ci"], &[]).status.success());
}

#[test]
fn managed_images_are_built_with_schema_metadata_and_failed_builds_never_push() {
	let root = project();
	succeeds(&cli(
		root.path(),
		&["infra", "build", "--version", "v1", "--push"],
		&[],
	));
	let calls = fs::read_to_string(root.path().join("docker-calls")).unwrap();
	assert!(calls.contains(&format!(
		"--label io.leptoskit.migrations={}",
		leptoskit_db::checksum("[]")
	)));
	assert!(calls.contains("push ghcr.io/team/notes:v1"));
	assert!(!calls.contains("fixture-registry-password"));
	fs::write(root.path().join("docker-calls"), "").unwrap();
	assert!(
		!cli(
			root.path(),
			&["infra", "build", "--version", "v2", "--push"],
			&[("MOCK_BUILD_EXIT", "1")]
		)
		.status
		.success()
	);
	assert!(
		!fs::read_to_string(root.path().join("docker-calls"))
			.unwrap()
			.contains("push")
	);
}

#[test]
fn image_with_different_migrations_is_rejected_before_starting_the_app() {
	let root = project();
	succeeds(&cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	let calls = root.path().join("infra/notes-production/kamal-calls");
	fs::write(&calls, "").unwrap();
	assert!(
		!cli(
			root.path(),
			&["deploy", "--version", "v2"],
			&[("MOCK_SCHEMA_LABEL", "wrong-checkout")]
		)
		.status
		.success()
	);
	assert!(!fs::read_to_string(calls).unwrap().contains("app boot"));
}

#[test]
fn existing_registry_tags_are_reused_explicitly_and_never_rebuilt() {
	let root = project();
	let environment = [("MOCK_REGISTRY_EXISTS", "1")];
	assert!(
		!cli(
			root.path(),
			&["infra", "build", "--version", "v1", "--push"],
			&environment
		)
		.status
		.success()
	);
	fs::write(root.path().join("docker-calls"), "").unwrap();
	succeeds(&cli(
		root.path(),
		&[
			"infra",
			"build",
			"--version",
			"v1",
			"--push",
			"--reuse-existing",
		],
		&environment,
	));
	let calls = fs::read_to_string(root.path().join("docker-calls")).unwrap();
	assert!(calls.contains("pull ghcr.io/team/notes:v1"));
	assert!(!calls.contains("build --platform") && !calls.contains("push ghcr.io"));
	assert!(
		!cli(
			root.path(),
			&[
				"infra",
				"build",
				"--version",
				"v1",
				"--push",
				"--reuse-existing"
			],
			&[
				("MOCK_REGISTRY_EXISTS", "1"),
				("MOCK_SCHEMA_LABEL", "wrong-checkout")
			]
		)
		.status
		.success()
	);
}

#[test]
fn removing_persistent_resource_blocks_cannot_bypass_destroy_protection() {
	let root = project();
	for (kind, name) in [
		("hcloud_server", "app"),
		("digitalocean_droplet", "app"),
		("cloudflare_r2_bucket", "backups"),
		("b2_bucket", "backups"),
		("hcloud_network", "app"),
	] {
		let plan = serde_json::json!({"format_version":"1.2", "resource_changes":[{"type": kind, "name":name, "change":{"actions":["delete","create"]}}]}).to_string();
		assert!(
			!cli(
				root.path(),
				&["infra", "plan"],
				&[("MOCK_PLAN_JSON", &plan)]
			)
			.status
			.success()
		);
		assert!(
			!root
				.path()
				.join("infra/notes-production/plan.sha256")
				.exists()
		);
	}
	let plan = r#"{"format_version":"1.2","resource_changes":[{"type":"hcloud_server","name":"replica","change":{"actions":["delete"]}}]}"#;
	succeeds(&cli(
		root.path(),
		&["infra", "plan"],
		&[("MOCK_PLAN_JSON", plan)],
	));
}

#[test]
fn lost_primary_recovery_requires_confirmation_and_preparation_never_overwrites_latest_backup() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "plan"], &[]));
	assert!(
		!cli(root.path(), &["infra", "apply"], &[("MOCK_SSH_EXIT", "1")])
			.status
			.success()
	);
	assert!(
		!cli(root.path(), &["infra", "apply", "--offline"], &[])
			.status
			.success()
	);
	succeeds(&cli(
		root.path(),
		&["infra", "apply", "--offline", "--yes"],
		&[("MOCK_SSH_EXIT", "1")],
	));
	succeeds(&cli(root.path(), &["infra", "prepare"], &[]));
	let calls = fs::read_to_string(root.path().join("infra/notes-production/kamal-calls")).unwrap();
	assert!(calls.contains("server bootstrap"));
	assert!(!calls.contains("app boot"));
	assert!(
		!root
			.path()
			.join("infra/notes-production/releases.json")
			.exists()
	);
}

#[test]
fn multi_host_releases_bind_shared_services_and_refuse_different_images() {
	let root = project();
	let config = CONFIG.replace(
		"name = \"notes-production\"",
		"name = \"notes-production\"\ninstances = 2",
	);
	fs::write(
		root.path().join("leptoskit.toml"),
		format!(
			"{config}{}",
			r"
[infra.services]
relay = true
cache = true
[rpc.live]
relay_url_env = 'NATS_URL'
"
		),
	)
	.unwrap();
	let hosts = "[\"192.0.2.10\",\"192.0.2.11\"]";
	succeeds(&cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[("MOCK_HOSTS", hosts)],
	));
	let directory = root.path().join("infra/notes-production");
	let runtime: serde_json::Value =
		serde_json::from_slice(&fs::read(directory.join("release.yml")).unwrap()).unwrap();
	assert_eq!(
		runtime["servers"]["web"]["hosts"].as_array().unwrap().len(),
		2
	);
	let references = fs::read_to_string(directory.join(".kamal/secrets")).unwrap();
	assert!(
		references.contains("NATS_URL=$NATS_URL") && references.contains("VALKEY_URL=$VALKEY_URL")
	);
	assert!(!references.contains("fixture-relay-token"));
	fs::write(directory.join("kamal-calls"), "").unwrap();
	assert!(
		!cli(
			root.path(),
			&["deploy", "--version", "v2"],
			&[
				("MOCK_HOSTS", hosts),
				(
					"MOCK_SECONDARY_DIGEST",
					"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
				)
			]
		)
		.status
		.success()
	);
	assert!(
		!fs::read_to_string(directory.join("kamal-calls"))
			.unwrap()
			.contains("app boot")
	);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paired_outbound_worker_executes_a_hosted_plan_with_private_state_bindings() {
	use leptoskit_deploy_console::hosted::Billing;
	use leptoskit_deploy_console::hosted::Hosted;
	use leptoskit_deploy_console::hosted::Ownership;
	use serde_json::json;
	let root = project();
	let cloud = tempfile::tempdir().unwrap();
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let url = format!("http://{}", listener.local_addr().unwrap());
	let host = Hosted::open(
		&cloud.path().join("broker"),
		"test-hosted-encryption-owner-key-0123456789".into(),
		&url,
		None,
	)
	.unwrap();
	let key = host.accounts().create("pilot", "cus_pilot").unwrap();
	let billing = Billing::new(
		"test".into(),
		"test-hook".into(),
		"price_control".into(),
		"price_team".into(),
	)
	.unwrap();
	let until = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_secs()
		+ 3600;
	billing.apply_subscription(&host,"pilot",&json!({"id":"sub_pilot","customer":"cus_pilot","status":"active","items":{"data":[{"quantity":1,"price":{"id":"price_control"},"current_period_end":until}]}})).unwrap();
	host.register(
		"pilot",
		"notes-production",
		"production",
		Ownership::Customer,
	)
	.unwrap();
	let code = host
		.accounts()
		.pairing("pilot", "notes-production")
		.unwrap();
	let server = tokio::spawn(async move {
		axum::serve(listener, host.router()).await.unwrap();
	});
	fs::write(
		root.path().join("leptoskit.toml"),
		CONFIG.replace("[infra]", "[infra]\nhosted_state=true"),
	)
	.unwrap();
	let tofu = root.path().join("bin/tofu");
	let source = fs::read_to_string(&tofu).unwrap();
	fs::write(
		&tofu,
		source.replace(
			"set -eu",
			"set -eu\nif [ \"$1\" = init ]; then test \"$TF_HTTP_USERNAME\" = worker; test -n \
			 \"$TF_HTTP_PASSWORD\"; test -n \"$TF_HTTP_LOCK_ADDRESS\"; fi",
		),
	)
	.unwrap();
	let data = root.path().join("worker");
	let path = std::env::join_paths(
		std::iter::once(root.path().join("bin"))
			.chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
	)
	.unwrap();
	let mut child = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(["console", "worker", "--url", &url, "--project"])
		.arg(root.path())
		.arg("--data")
		.arg(&data)
		.env("LEPTOSKIT_PAIRING_CODE", code)
		.env("PATH", path)
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::null())
		.spawn()
		.unwrap();
	let _guard = ConsoleProcess(&mut child);
	succeeds(&cli(
		root.path(),
		&["console", "connect", "--url", &url],
		&[
			("LEPTOSKIT_CONTROL_KEY", &key),
			("XDG_CONFIG_HOME", root.path().to_str().unwrap()),
		],
	));
	succeeds(&cli(
		root.path(),
		&["infra", "plan"],
		&[("LEPTOSKIT_CONTROL_KEY", &key)],
	));
	let resources: serde_json::Value = serde_json::from_slice(
		&fs::read(root.path().join("infra/notes-production/main.tf.json")).unwrap(),
	)
	.unwrap();
	assert_eq!(resources["terraform"]["backend"]["http"], json!({}));
	let credential = fs::read_to_string(data.join("worker.json")).unwrap();
	assert!(credential.contains("pilot."));
	assert_eq!(
		fs::metadata(data.join("worker.json"))
			.unwrap()
			.permissions()
			.mode()
			& 0o077,
		0
	);
	let status = cli(
		root.path(),
		&["console", "status"],
		&[("LEPTOSKIT_CONTROL_KEY", &key)],
	);
	succeeds(&status);
	assert!(String::from_utf8_lossy(&status.stdout).contains("succeeded"));
	server.abort();
}

#[test]
fn releases_validate_shared_history_and_hosts_before_mutating_anything() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	let too_many = serde_json::to_string(&vec!["192.0.2.1"; 17]).unwrap();
	for hosts in ["[]", "[\"not-an-ip\"]", too_many.as_str()] {
		fails_with(
			&cli(
				root.path(),
				&["deploy", "--setup", "--version", "v1"],
				&[("MOCK_HOSTS", hosts)],
			),
			"invalid applied application hosts",
		);
	}
	fails_with(
		&cli(
			root.path(),
			&["deploy", "--setup", "--version", "v1"],
			&[("MOCK_OUTPUT_FAIL", "1")],
		),
		"couldn't read application hosts from applied infrastructure",
	);
	fails_with(
		&cli(
			root.path(),
			&["infra", "releases"],
			&[("MOCK_OUTPUT_RAW_FAIL", "1")],
		),
		"couldn't read infrastructure output `server_ip`",
	);
	fails_with(
		&cli(
			root.path(),
			&["infra", "releases"],
			&[("MOCK_SERVER_IP", "not-an-address")],
		),
		"invalid server_ip output",
	);
	succeeds(&cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	let history = root
		.path()
		.join("remote/state/notes-production/control/releases.json");
	fs::write(&history, "not json").unwrap();
	fails_with(
		&cli(root.path(), &["infra", "releases"], &[]),
		"invalid shared release history",
	);
	fails_with(
		&cli(root.path(), &["deploy", "--version", "v2"], &[]),
		"invalid shared release history",
	);
}

#[test]
fn release_tags_require_setup_recorded_versions_and_maintenance_for_new_schema() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	fails_with(
		&cli(root.path(), &["deploy", "--version", "v1"], &[]),
		"first release requires --setup",
	);
	fails_with(
		&cli(root.path(), &["deploy"], &[]),
		"pass --version or --rollback",
	);
	fails_with(
		&cli(root.path(), &["deploy", "--version", "latest"], &[]),
		"use a unique immutable image tag",
	);
	fails_with(
		&cli(root.path(), &["deploy", "--version", "v/1"], &[]),
		"use a unique immutable image tag",
	);
	fails_with(
		&cli(root.path(), &["deploy", "--rollback", "v1"], &[]),
		"version is absent from shared release history",
	);
	succeeds(&cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	fails_with(
		&cli(root.path(), &["deploy", "--version", "v1"], &[]),
		"tag already recorded",
	);
	let checksum = add_migration(root.path(), "new_schema");
	fails_with(
		&cli(root.path(), &["deploy", "--version", "v2"], &[]),
		"schema changes require --maintenance",
	);
	let pairs = serde_json::to_string(&vec![("new_schema".to_owned(), checksum.clone())]).unwrap();
	let label = leptoskit_db::checksum(&pairs);
	let remote_history = format!("[[\"new_schema\",\"{checksum}\"]]");
	succeeds(&cli(
		root.path(),
		&["deploy", "--version", "v2", "--maintenance"],
		&[
			("MOCK_SCHEMA_LABEL", &label),
			("MOCK_HISTORY", &remote_history),
		],
	));
	assert_eq!(release_count(root.path()), 2);
}

#[test]
fn missing_secrets_and_database_drift_stop_before_any_application_boot() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	for (variable, value, expected) in [
		(
			"POSTGRES_PASSWORD",
			"",
			"set required secret POSTGRES_PASSWORD",
		),
		(
			"DATABASE_PASSWORD",
			"fixture-password",
			"administrator and application database passwords must differ",
		),
		(
			"DATABASE_PASSWORD",
			"",
			"set required secret DATABASE_PASSWORD",
		),
		(
			"KAMAL_REGISTRY_PASSWORD",
			"",
			"set required secret KAMAL_REGISTRY_PASSWORD",
		),
	] {
		fails_with(
			&cli(
				root.path(),
				&["deploy", "--setup", "--version", "v1"],
				&[(variable, value)],
			),
			expected,
		);
	}

	let custom = configured(&CONFIG.replace("[infra]", "[infra]\nsecrets = [\"EXTRA_SECRET\"]"));
	succeeds(&cli(custom.path(), &["infra", "generate"], &[]));
	fails_with(
		&cli(
			custom.path(),
			&["deploy", "--setup", "--version", "v1"],
			&[],
		),
		"set required secret EXTRA_SECRET",
	);

	let analytics = configured(&format!("{CONFIG}\n[infra.plugins.analytics]\n"));
	succeeds(&cli(analytics.path(), &["infra", "generate"], &[]));
	fails_with(
		&cli(
			analytics.path(),
			&["deploy", "--setup", "--version", "v1"],
			&[("LEPTOSKIT_ANALYTICS_READ_KEY", "")],
		),
		"set required secret LEPTOSKIT_ANALYTICS_READ_KEY",
	);
	fails_with(
		&cli(
			analytics.path(),
			&["deploy", "--setup", "--version", "v1"],
			&[("LEPTOSKIT_ANALYTICS_READ_KEY", "short")],
		),
		"LEPTOSKIT_ANALYTICS_READ_KEY needs 32..=256 printable non-space ASCII characters",
	);

	let services = configured(&format!(
		"{CONFIG}\n[infra.services]\ncache = true\nrelay = true\n[rpc.live]\nrelay_url_env = \
		 'NATS_URL'\n"
	));
	succeeds(&cli(services.path(), &["infra", "generate"], &[]));
	fails_with(
		&cli(
			services.path(),
			&["deploy", "--setup", "--version", "v1"],
			&[("VALKEY_PASSWORD", "")],
		),
		"set required secret VALKEY_PASSWORD",
	);
	fails_with(
		&cli(
			services.path(),
			&["deploy", "--setup", "--version", "v1"],
			&[("NATS_TOKEN", "")],
		),
		"set required secret NATS_TOKEN",
	);

	let drift = project();
	succeeds(&cli(drift.path(), &["infra", "generate"], &[]));
	succeeds(&cli(
		drift.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	fails_with(
		&cli(
			drift.path(),
			&["deploy", "--version", "v2"],
			&[("MOCK_HISTORY", "[[\"unknown\",\"checksum\"]]")],
		),
		"database drifted from shared history",
	);
	fails_with(
		&cli(
			drift.path(),
			&["deploy", "--version", "v2"],
			&[("MOCK_HISTORY_FAIL", "1")],
		),
		"couldn't verify remote migration history; rollback is blocked",
	);
}

#[test]
fn a_failed_boot_restores_the_previous_release_or_reports_that_it_could_not() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	succeeds(&cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	fails_with(
		&cli(
			root.path(),
			&["deploy", "--version", "v2"],
			&[("MOCK_KAMAL_FAIL_VERSION", "v2")],
		),
		"Kamal operation failed",
	);
	assert_eq!(
		fs::read_to_string(root.path().join("infra/notes-production/boot-versions")).unwrap(),
		"v1\nv2\nv1\n"
	);
	assert_eq!(operation(root.path())["phase"], "previous-restored");
	assert_eq!(release_count(root.path()), 1);

	let abandoned = project();
	succeeds(&cli(abandoned.path(), &["infra", "generate"], &[]));
	succeeds(&cli(
		abandoned.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	let changed = "b".repeat(64);
	fails_with(
		&cli(
			abandoned.path(),
			&["deploy", "--version", "v2"],
			&[
				("MOCK_KAMAL_FAIL_VERSION", "v2"),
				("MOCK_DIGEST", changed.as_str()),
			],
		),
		"Automatic recovery did not complete",
	);
	assert_eq!(release_count(abandoned.path()), 1);
}

#[test]
fn verification_failures_stop_the_app_and_keep_the_recorded_history() {
	let root = project();
	let two_hosts = format!(
		"{}\n[infra.services]\nrelay = true\n[rpc.live]\nrelay_url_env = 'NATS_URL'\n",
		CONFIG.replace(
			"name = \"notes-production\"",
			"name = \"notes-production\"\ninstances = 2",
		)
	);
	fs::write(root.path().join("leptoskit.toml"), two_hosts).unwrap();
	let hosts = "[\"192.0.2.10\",\"192.0.2.11\"]";
	succeeds(&cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[("MOCK_HOSTS", hosts)],
	));
	let secondary = "b".repeat(64);
	fails_with(
		&cli(
			root.path(),
			&["deploy", "--version", "v2"],
			&[
				("MOCK_HOSTS", hosts),
				("MOCK_CONTAINER_SECONDARY_DIGEST", secondary.as_str()),
			],
		),
		"hosts are running different image digests",
	);
	assert_eq!(
		operation(root.path())["phase"],
		"verification-failed-app-stopped"
	);
	assert_eq!(release_count(root.path()), 1);

	let rollback = project();
	succeeds(&cli(rollback.path(), &["infra", "generate"], &[]));
	succeeds(&cli(
		rollback.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	let container = "b".repeat(64);
	fails_with(
		&cli(
			rollback.path(),
			&["deploy", "--rollback", "v1"],
			&[("MOCK_CONTAINER_DIGEST", container.as_str())],
		),
		"running image differs from recorded digest",
	);
	assert_eq!(release_count(rollback.path()), 1);
}

#[test]
fn preparing_services_validates_hosts_and_reuses_running_accessories() {
	let single = project();
	succeeds(&cli(single.path(), &["infra", "generate"], &[]));
	fails_with(
		&cli(
			single.path(),
			&["infra", "prepare"],
			&[("MOCK_PRIVATE_IP", "not-an-address")],
		),
		"invalid private IP",
	);

	let paired = configured(&format!(
		"{}\n[infra.services]\nrelay = true\n[rpc.live]\nrelay_url_env = 'NATS_URL'\n",
		CONFIG.replace(
			"name = \"notes-production\"",
			"name = \"notes-production\"\ninstances = 2",
		)
	));
	succeeds(&cli(paired.path(), &["infra", "generate"], &[]));
	fails_with(
		&cli(paired.path(), &["infra", "prepare"], &[]),
		"apply the configured hosts before preparing services",
	);
	succeeds(&cli(
		paired.path(),
		&["infra", "prepare"],
		&[
			("MOCK_HOSTS", "[\"192.0.2.10\",\"192.0.2.11\"]"),
			("MOCK_ACCESSORY_STATUS", "running"),
		],
	));
	let calls =
		fs::read_to_string(paired.path().join("infra/notes-production/kamal-calls")).unwrap();
	assert!(calls.contains("server bootstrap"));
	assert!(
		!calls.contains("accessory boot") && !calls.contains("accessory start"),
		"running accessories are left alone: {calls}"
	);
	assert!(!calls.contains("app boot"));
}

#[test]
fn external_infrastructure_is_generated_inspected_and_never_deployed() {
	let root = configured(&format!(
		r#"
[app]
name = "notes"
render = "ssr"
[db]
dialect = "postgres"
[infra]
name = "notes-production"
domain = "app.example.com"
compute = "digitalocean"
location = "ams3"
server_type = "s-1vcpu-1gb"
ssh_key_ids = [123]
ssh_cidrs = ["192.0.2.1/32"]
account_id = "{}"
zone_id = "{}"
backup_bucket = "notes-backups"
[infra.external]
origin_port = 4701
[infra.backups]
enabled = false
"#,
		"a".repeat(32),
		"b".repeat(32)
	));
	let generated = cli(root.path(), &["infra", "generate"], &[]);
	succeeds(&generated);
	assert!(
		String::from_utf8_lossy(&generated.stdout).contains("run `lk infra plan`"),
		"{}",
		String::from_utf8_lossy(&generated.stdout)
	);
	assert!(!root.path().join("Dockerfile").exists());
	succeeds(&cli(root.path(), &["infra", "plugins"], &[]));
	let output = cli(root.path(), &["infra", "output"], &[]);
	succeeds(&output);
	let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
	assert_eq!(value["runtime"], "external");
	assert_eq!(value["backup_bucket"], "notes-backups");
	assert!(value["updates_bucket"].is_null());
	let status = cli(root.path(), &["infra", "status"], &[]);
	succeeds(&status);
	let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
	assert_eq!(value["runtime"], "external");
	assert_eq!(
		value["application_health"],
		"inspect through the service supervisor"
	);
	succeeds(&cli(root.path(), &["infra", "plan"], &[]));
	succeeds(&cli(root.path(), &["infra", "apply"], &[]));
	fails_with(
		&cli(root.path(), &["deploy", "--version", "v1"], &[]),
		"deploy through the service supervisor",
	);
	fails_with(
		&cli(root.path(), &["infra", "prepare"], &[]),
		"infra.external manages provisioning only",
	);
	fails_with(
		&cli(root.path(), &["infra", "releases"], &[]),
		"infra.external manages provisioning only",
	);
}

#[test]
fn code_push_publishing_requires_a_native_platform_and_the_provisioned_host() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	fails_with(
		&cli(root.path(), &["infra", "publish", "--platform", "web"], &[]),
		"the managed publisher supports iOS and Android code push",
	);
	fails_with(
		&cli(root.path(), &["infra", "publish", "--platform", "ios"], &[]),
		"codepush.server must match the provisioned updates hostname",
	);
	fs::write(
		root.path().join("leptoskit.toml"),
		format!("{CONFIG}\n[codepush]\nserver = \"https://updates.example.com\"\n"),
	)
	.unwrap();
	succeeds(&cli(root.path(), &["infra", "generate", "--force"], &[]));
	let published = cli(root.path(), &["infra", "publish", "--platform", "ios"], &[]);
	fails_with(&published, "publishing needs the bucket's S3 credentials");
}

#[test]
fn backups_validate_uploads_and_require_a_postgres_archive() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	fails_with(
		&cli(
			root.path(),
			&["infra", "backup", "--out", "upload.dump", "--upload"],
			&[],
		),
		"publishing needs the bucket's S3 credentials",
	);
	fails_with(
		&cli(
			root.path(),
			&["infra", "backup", "--out", "archive.dump"],
			&[("MOCK_PG_DUMP", "NOPEs")],
		),
		"backup did not produce a Postgres custom archive",
	);
	assert!(!root.path().join("archive.dump").exists());
	let artifacts = root.path().join("artifacts");
	fs::create_dir(&artifacts).unwrap();
	succeeds(&cli(
		root.path(),
		&["infra", "backup", "--out", "artifacts/db.dump"],
		&[],
	));
	assert!(artifacts.join("db.dump").is_file());

	// A B2 backup endpoint is the region's S3 host, not R2's.
	let endpoint = "https://s3.us-west-004.backblazeb2.com";
	let b2 = configured(&format!("{CONFIG}\n[infra.b2]\nregion = \"us-west-004\"\n"));
	succeeds(&cli(b2.path(), &["infra", "generate"], &[]));
	fails_with(
		&cli(
			b2.path(),
			&["infra", "backup", "--out", "upload.dump", "--upload"],
			&[("MOCK_BACKUP_ENDPOINT", endpoint)],
		),
		"publishing needs the bucket's S3 credentials",
	);
}

#[test]
fn saved_plans_reject_untrusted_metadata_and_never_apply_it() {
	let root = project();
	fails_with(
		&cli(root.path(), &["infra", "plan"], &[("MOCK_SHOW_EXIT", "1")]),
		"couldn't inspect the saved plan for protected data resources",
	);
	for (plan, expected) in [
		(
			r#"{"format_version":"2.0","resource_changes":[]}"#,
			"unsupported plan JSON format",
		),
		(
			r#"{"format_version":"1.2","resource_changes":[{"name":"app","change":{"actions":["delete"]}}]}"#,
			"invalid planned resource type",
		),
		(
			r#"{"format_version":"1.2","resource_changes":[{"type":"hcloud_server","change":{"actions":["delete"]}}]}"#,
			"invalid planned resource name",
		),
		(
			r#"{"format_version":"1.2","resource_changes":[{"type":"hcloud_server","name":"app"}]}"#,
			"invalid planned resource actions",
		),
	] {
		fails_with(
			&cli(root.path(), &["infra", "plan"], &[("MOCK_PLAN_JSON", plan)]),
			expected,
		);
	}
	let directory = root.path().join("infra/notes-production");
	fs::create_dir_all(directory.join("plan.sha256")).unwrap();
	fails_with(
		&cli(root.path(), &["infra", "plan"], &[]),
		"couldn't invalidate the previous plan",
	);
	fs::remove_dir(directory.join("plan.sha256")).unwrap();
	succeeds(&cli(root.path(), &["infra", "plan"], &[]));
	fails_with(
		&cli(
			root.path(),
			&["infra", "apply"],
			&[("MOCK_OUTPUT_FAIL", "1")],
		),
		"couldn't inspect applied state before reserving the environment",
	);
	for (json, expected) in [
		(
			r#"{"server_ip":{"value":"not-an-address"}}"#,
			"invalid primary IP in applied state",
		),
		(r#"{"server_ip":{"value":7}}"#, "invalid primary output"),
	] {
		fails_with(
			&cli(
				root.path(),
				&["infra", "apply"],
				&[("MOCK_OUTPUT_JSON", json)],
			),
			expected,
		);
	}
	succeeds(&cli(root.path(), &["infra", "apply"], &[]));
}

#[test]
fn offers_require_tokens_and_a_positive_minimum_and_budgets_never_guess_prices() {
	let root = configured(&format!(
		"{CONFIG}\n[infra.budget]\ncurrency = \"EUR\"\nmonthly_compute = 10.0\n"
	));
	fails_with(
		&cli(root.path(), &["infra", "offers"], &[]),
		"set HCLOUD_TOKEN and/or DIGITALOCEAN_TOKEN to compare current offers",
	);
	fails_with(
		&cli(
			root.path(),
			&["infra", "offers", "--min-memory-gb", "0"],
			&[],
		),
		"minimum memory must be positive",
	);
	fails_with(
		&cli(root.path(), &["infra", "plan"], &[]),
		"set HCLOUD_TOKEN to read current provider offers",
	);
	let digitalocean = configured(&format!(
		"{}\n[infra.budget]\ncurrency = \"USD\"\nmonthly_compute = 10.0\n",
		CONFIG
			.replace(
				"location = \"hel1\"",
				"compute = \"digitalocean\"\nlocation = \"ams3\""
			)
			.replace("server_type = \"cx23\"", "server_type = \"s-1vcpu-1gb\"")
	));
	fails_with(
		&cli(digitalocean.path(), &["infra", "plan"], &[]),
		"set DIGITALOCEAN_TOKEN to read current provider offers",
	);
}

#[test]
fn analytics_requires_an_enabled_plugin_and_a_read_key() {
	let plain = project();
	fails_with(
		&cli(plain.path(), &["infra", "analytics"], &[]),
		"enable [infra.plugins.analytics] first",
	);
	let enabled = configured(&format!("{CONFIG}\n[infra.plugins.analytics]\n"));
	fails_with(
		&cli(enabled.path(), &["infra", "analytics"], &[]),
		"set LEPTOSKIT_ANALYTICS_READ_KEY",
	);
	fails_with(
		&cli(
			enabled.path(),
			&["infra", "analytics"],
			&[("LEPTOSKIT_ANALYTICS_READ_KEY", "short")],
		),
		"invalid analytics read key",
	);
}

#[test]
fn generated_dockerignore_keeps_managed_state_out_of_images() {
	let root = project();
	fs::write(root.path().join(".dockerignore"), "target\n").unwrap();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	let ignore = fs::read_to_string(root.path().join(".dockerignore")).unwrap();
	assert!(ignore.starts_with("target\n"), "{ignore}");
	assert_eq!(ignore.matches("\n/infra\n").count(), 1, "{ignore}");
	assert!(ignore.contains("/.kamal"), "{ignore}");
}

#[test]
fn releases_refuse_projects_without_readiness_probes() {
	let root = configured(&CONFIG.replace("probes = true", "probes = false"));
	// The fixture contains a `readyz` comment; only the declaration counts.
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	fails_with(
		&cli(root.path(), &["deploy", "--setup", "--version", "v1"], &[]),
		"declare serve.probes = true",
	);
}

#[test]
fn releases_prefix_the_configured_registry_once() {
	let root =
		configured(&CONFIG.replace("image = \"ghcr.io/team/notes\"", "image = \"team/notes\""));
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	succeeds(&cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	let calls = fs::read_to_string(root.path().join("infra/notes-production/kamal-calls")).unwrap();
	assert!(calls.contains("build pull --version v1"));
	let remote = fs::read_to_string(root.path().join("remote/ssh-calls")).unwrap();
	assert!(remote.contains("ghcr.io/team/notes:v1"), "{remote}");
	assert!(!remote.contains("ghcr.io/ghcr.io/"), "{remote}");
}

#[test]
fn restores_refuse_unfinished_relay_recovery_before_starting_applications() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	succeeds(&cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	let reset = root
		.path()
		.join("remote/state/notes-production/control/derived-reset-required.json");
	fs::write(&reset, "{}").unwrap();
	fails_with(
		&cli(root.path(), &["deploy", "--version", "v2"], &[]),
		"database restore has unfinished relay/cache recovery",
	);
}

#[test]
fn verification_rechecks_the_database_history_after_boot() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	succeeds(&cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	fs::remove_file(root.path().join("remote/history-count")).unwrap();
	fails_with(
		&cli(
			root.path(),
			&["deploy", "--version", "v2"],
			&[("MOCK_HISTORY_AFTER", "[[\"unknown\",\"checksum\"]]")],
		),
		"database does not match release; shared history was not advanced",
	);
	assert_eq!(
		operation(root.path())["phase"],
		"verification-failed-app-stopped"
	);
	assert_eq!(release_count(root.path()), 1);
}

#[test]
fn recovery_stops_the_app_when_the_history_no_longer_matches() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	succeeds(&cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	fs::remove_file(root.path().join("remote/history-count")).unwrap();
	fs::write(root.path().join("infra/notes-production/kamal-calls"), "").unwrap();
	fails_with(
		&cli(
			root.path(),
			&["deploy", "--version", "v2"],
			&[
				("MOCK_KAMAL_FAIL_VERSION", "v2"),
				("MOCK_HISTORY_AFTER", "[[\"unknown\",\"checksum\"]]"),
			],
		),
		"Kamal operation failed",
	);
	let calls = fs::read_to_string(root.path().join("infra/notes-production/kamal-calls")).unwrap();
	assert!(
		!calls.contains("app boot --version v1"),
		"a drifted database must not be handed the previous release: {calls}"
	);
	assert_eq!(release_count(root.path()), 1);
}

#[test]
fn releases_check_the_host_count_and_private_address_before_boot() {
	let paired = configured(&format!(
		"{}\n[infra.services]\nrelay = true\n[rpc.live]\nrelay_url_env = 'NATS_URL'\n",
		CONFIG.replace(
			"name = \"notes-production\"",
			"name = \"notes-production\"\ninstances = 2",
		)
	));
	succeeds(&cli(paired.path(), &["infra", "generate"], &[]));
	fails_with(
		&cli(
			paired.path(),
			&["deploy", "--setup", "--version", "v1"],
			&[],
		),
		"apply the configured host count before release",
	);
	let single = project();
	succeeds(&cli(single.path(), &["infra", "generate"], &[]));
	fails_with(
		&cli(
			single.path(),
			&["deploy", "--setup", "--version", "v1"],
			&[("MOCK_PRIVATE_IP", "not-an-address")],
		),
		"invalid primary private address",
	);
}

#[test]
fn rollback_schema_refuses_live_clients_and_stale_prefixes() {
	let busy = project();
	succeeds(&cli(busy.path(), &["infra", "generate"], &[]));
	succeeds(&cli(
		busy.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	fails_with(
		&cli(
			busy.path(),
			&["deploy", "--rollback", "v1", "--rollback-schema", "--yes"],
			&[("MOCK_DB_CLIENTS", "1")],
		),
		"database still has clients; stop external writers before down migrations",
	);

	let stale = project();
	let checksum = add_migration(stale.path(), "first");
	let pairs = serde_json::to_string(&vec![("first".to_owned(), checksum.clone())]).unwrap();
	let label = leptoskit_db::checksum(&pairs);
	let history = format!("[[\"first\",\"{checksum}\"]]");
	succeeds(&cli(
		stale.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[("MOCK_SCHEMA_LABEL", &label), ("MOCK_HISTORY", &history)],
	));
	fs::write(
		stale.path().join("migrations/first/migration.sql"),
		"SELECT 2;",
	)
	.unwrap();
	fails_with(
		&cli(
			stale.path(),
			&["deploy", "--rollback", "v1", "--rollback-schema", "--yes"],
			&[("MOCK_SCHEMA_LABEL", &label)],
		),
		"target migration prefix is not present in this checkout",
	);
}

#[test]
fn a_schema_rollback_without_a_reachable_database_reports_the_tunnel() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	succeeds(&cli(
		root.path(),
		&["deploy", "--setup", "--version", "v1"],
		&[],
	));
	fails_with(
		&cli(
			root.path(),
			&["deploy", "--rollback", "v1", "--rollback-schema", "--yes"],
			&[],
		),
		"the database can't be reached",
	);
}

#[test]
fn managed_settings_require_postgres_and_a_matching_relay_url() {
	let sqlite = configured(&CONFIG.replace("dialect = \"postgres\"", "dialect = \"sqlite\""));
	fails_with(
		&cli(sqlite.path(), &["infra", "plan"], &[]),
		"the managed release preset requires [db] dialect = 'postgres'",
	);
	let relay = configured(&format!("{CONFIG}\n[infra.services]\nrelay = true\n"));
	fails_with(
		&cli(relay.path(), &["infra", "plan"], &[]),
		"configure rpc.live.relay_url_env together with infra.services.relay",
	);
}

#[test]
fn tofu_must_exist_and_succeed_before_a_plan_is_saved() {
	let missing = project();
	succeeds(&cli(missing.path(), &["infra", "generate"], &[]));
	fs::remove_file(missing.path().join("bin/tofu")).unwrap();
	fails_with(
		&cli(missing.path(), &["infra", "plan"], &[]),
		"couldn't run tofu; install it on the operator/CI machine",
	);
	let failing = project();
	succeeds(&cli(failing.path(), &["infra", "generate"], &[]));
	fails_with(
		&cli(
			failing.path(),
			&["infra", "plan"],
			&[("MOCK_TOFU_INIT_EXIT", "1")],
		),
		"tofu failed",
	);
}

#[test]
fn applied_bindings_must_match_the_configuration() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	fs::write(
		root.path().join("leptoskit.toml"),
		CONFIG.replace(
			"domain = \"app.example.com\"",
			"domain = \"other.example.com\"",
		),
	)
	.unwrap();
	succeeds(&cli(root.path(), &["infra", "generate", "--force"], &[]));
	fails_with(
		&cli(root.path(), &["deploy", "--setup", "--version", "v1"], &[]),
		"app_url differs from applied infrastructure",
	);
}

#[test]
fn extra_tofu_files_change_the_saved_plan_fingerprint() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	let extra = root.path().join("infra/notes-production/extra.tfvars");
	fs::write(&extra, "servers = 1\n").unwrap();
	succeeds(&cli(root.path(), &["infra", "plan"], &[]));
	fs::write(&extra, "servers = 2\n").unwrap();
	fails_with(
		&cli(root.path(), &["infra", "apply"], &[]),
		"the configuration or plan changed; run `lk infra plan` again",
	);
}

#[test]
fn a_backup_transport_failure_never_publishes_a_file() {
	let root = project();
	succeeds(&cli(root.path(), &["infra", "generate"], &[]));
	fails_with(
		&cli(
			root.path(),
			&["infra", "backup", "--out", "broken.dump"],
			&[("MOCK_PG_DUMP_EXIT", "1")],
		),
		"backup failed; no backup file was published",
	);
	assert!(!root.path().join("broken.dump").exists());
}

fn verified_descriptor(root: &Path) -> (String, String) {
	fs::write(
		root.join("leptoskit.toml"),
		CONFIG.replace(
			"render = \"ssr\"",
			"render = \"ssr\"\nplatforms = [\"server\"]\ndefault_platform = \"server\"",
		),
	)
	.unwrap();
	let descriptor = serde_json::json!({
		"format_version": 1, "kind": "leptoskit-verified-backend-image",
		"inventory_sha256": "a".repeat(64),
		"recipe": {"repository": "https://github.com/example/backend", "commit": "a".repeat(40),
			"builder_image": format!("builder@sha256:{}", "a".repeat(64)),
			"target": "aarch64-unknown-linux-musl", "package": "notes", "binary": "notes",
			"features": [], "no_default_features": false, "rustflags": []},
		"executable_sha256": "b".repeat(64), "executable_mode": 493,
		"migrations": leptoskit_db::checksum("[]"),
		"runtime_image": format!("runtime@sha256:{}", "a".repeat(64)),
		"runtime_config_sha256": "c".repeat(64), "engine_image_id": format!("sha256:{}", "a".repeat(64)),
		"image_config_sha256": leptoskit_db::checksum("{}"), "platform": "linux/arm64",
		"assets": "not_applicable", "services": "unsupported",
		"rpc_contract": {"state": "not_applicable", "declaration": "operator_declares_no_rpc"},
		"native_guest_compatibility": "not_applicable", "runtime_attestation": "absent"
	});
	let text = serde_json::to_string(&descriptor).unwrap();
	let path = root.join("descriptor.json");
	fs::write(&path, &text).unwrap();
	(path.to_str().unwrap().into(), leptoskit_db::checksum(&text))
}

#[test]
fn verified_release_command_enforces_evidence_before_and_after_boot() {
	let root = project();
	let (path, pin) = verified_descriptor(root.path());
	let args = [
		"deploy",
		"--setup",
		"--version",
		"v1",
		"--verified-descriptor",
		&path,
		"--descriptor-sha256",
		&pin,
	];
	fails_with(
		&cli(root.path(), &args, &[("MOCK_IMAGE_MISMATCH", "all")]),
		"remote image differs",
	);
	let calls = fs::read_to_string(root.path().join("infra/notes-production/kamal-calls")).unwrap();
	assert!(!calls.contains("proxy boot"));
	assert!(!calls.contains("app boot"));
	assert!(
		!root
			.path()
			.join("remote/state/notes-production/control/releases.json")
			.exists()
	);
	fails_with(
		&cli(root.path(), &args, &[("MOCK_IMAGE_MISMATCH", "postboot")]),
		"remote image differs",
	);
	assert_eq!(
		operation(root.path())["phase"],
		"verification-failed-app-stopped"
	);
	assert!(
		!root
			.path()
			.join("remote/state/notes-production/control/releases.json")
			.exists()
	);
	succeeds(&cli(root.path(), &args, &[]));
	for flags in [
		vec!["deploy", "--rollback", "v1"],
		vec!["deploy", "--recover", "--version", "v1"],
	] {
		fails_with(
			&cli(root.path(), &flags, &[("MOCK_IMAGE_MISMATCH", "all")]),
			"remote image differs",
		);
		assert_eq!(release_count(root.path()), 1);
		fails_with(
			&cli(root.path(), &flags, &[("MOCK_IMAGE_MISMATCH", "postboot")]),
			"remote image differs",
		);
		assert_eq!(
			operation(root.path())["phase"],
			"verification-failed-app-stopped"
		);
		assert_eq!(release_count(root.path()), 1);
	}
	fails_with(
		&cli(root.path(), &["deploy", "--version", "v2"], &[]),
		"cannot downgrade",
	);
	fs::write(root.path().join("remote/ssh-calls"), "").unwrap();
	let next = [
		"deploy",
		"--version",
		"v2",
		"--verified-descriptor",
		&path,
		"--descriptor-sha256",
		&pin,
	];
	fails_with(
		&cli(
			root.path(),
			&next,
			&[
				("MOCK_KAMAL_FAIL_VERSION", "v2"),
				("MOCK_IMAGE_MISMATCH", "postboot"),
			],
		),
		"Automatic recovery did not complete",
	);
	assert_eq!(release_count(root.path()), 1);
	let calls = fs::read_to_string(root.path().join("remote/ssh-calls")).unwrap();
	assert!(calls.contains("docker stop"));
	succeeds(&cli(
		root.path(),
		&["deploy", "--recover", "--version", "v1"],
		&[],
	));
	let history: serde_json::Value = serde_json::from_slice(
		&fs::read(
			root.path()
				.join("remote/state/notes-production/control/releases.json"),
		)
		.unwrap(),
	)
	.unwrap();
	assert_eq!(history[0]["evidence"], history[1]["evidence"]);
	assert!(history[1]["evidence"].is_object());
	let history_path = root
		.path()
		.join("remote/state/notes-production/control/releases.json");
	let mut legacy = history[0].clone();
	legacy["version"] = "legacy".into();
	legacy.as_object_mut().unwrap().remove("evidence");
	let mixed = serde_json::json!([legacy, history[0]]);
	fs::write(&history_path, serde_json::to_vec(&mixed).unwrap()).unwrap();
	for flags in [
		vec!["deploy", "--rollback", "legacy"],
		vec!["deploy", "--recover", "--version", "legacy"],
	] {
		fails_with(&cli(root.path(), &flags, &[]), "cannot downgrade");
		assert_eq!(
			fs::read(&history_path).unwrap(),
			serde_json::to_vec(&mixed).unwrap()
		);
	}
	let mut discarded = history.clone();
	discarded[1].as_object_mut().unwrap().remove("evidence");
	fs::write(&history_path, serde_json::to_vec(&discarded).unwrap()).unwrap();
	fails_with(
		&cli(
			root.path(),
			&["deploy", "--recover", "--version", "v1"],
			&[],
		),
		"discarded verified evidence",
	);
	let mut unknown = history.clone();
	unknown[0]["future_evidence"] = serde_json::json!({"verified": true});
	fs::write(&history_path, serde_json::to_vec(&unknown).unwrap()).unwrap();
	fails_with(
		&cli(
			root.path(),
			&["deploy", "--recover", "--version", "v1"],
			&[],
		),
		"invalid shared release history",
	);
	assert_eq!(
		fs::read(&history_path).unwrap(),
		serde_json::to_vec(&unknown).unwrap()
	);
}

#[test]
fn verified_descriptor_structure_is_rejected_at_command_boundary() {
	let root = project();
	let (path, _) = verified_descriptor(root.path());
	let valid: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
	for (pointer, bad) in [
		("/inventory_sha256", "bad"),
		("/executable_sha256", "bad"),
		("/migrations", "bad"),
		("/runtime_config_sha256", "bad"),
		("/image_config_sha256", "bad"),
		("/engine_image_id", "sha256:no"),
		("/runtime_image", "runtime:latest"),
		("/platform", "linux/amd64"),
		("/recipe/commit", "main"),
		("/recipe/repository", "file:///tmp/source"),
		("/recipe/builder_image", "builder:latest"),
		("/recipe/target", "unknown"),
	] {
		let mut value = valid.clone();
		*value.pointer_mut(pointer).unwrap() = bad.into();
		let text = serde_json::to_string(&value).unwrap();
		fs::write(&path, &text).unwrap();
		let output = cli(
			root.path(),
			&[
				"deploy",
				"--setup",
				"--version",
				"v1",
				"--verified-descriptor",
				&path,
				"--descriptor-sha256",
				&leptoskit_db::checksum(&text),
			],
			&[],
		);
		assert!(!output.status.success(), "accepted {pointer}");
		assert!(
			!root
				.path()
				.join("infra/notes-production/kamal-calls")
				.exists()
		);
	}
}

#[test]
fn verified_deploy_rejects_websites_and_services() {
	let root = project();
	let (path, pin) = verified_descriptor(root.path());
	let backend = fs::read_to_string(root.path().join("leptoskit.toml")).unwrap();
	for config in [
		CONFIG.to_owned(),
		format!("{backend}\n[infra.services]\ncache = true\n"),
	] {
		fs::write(root.path().join("leptoskit.toml"), config).unwrap();
		let output = cli(
			root.path(),
			&[
				"deploy",
				"--setup",
				"--version",
				"v1",
				"--verified-descriptor",
				&path,
				"--descriptor-sha256",
				&pin,
			],
			&[],
		);
		assert!(!output.status.success());
		assert!(
			!root
				.path()
				.join("infra/notes-production/kamal-calls")
				.exists()
		);
	}
}

fn backend_authorization(root: &Path, descriptor: &str, pin: &str) {
	use leptoskit_codepush::ed25519_dalek::SigningKey;
	let public = |seed| {
		hex::encode(
			SigningKey::from_bytes(&[seed; 32])
				.verifying_key()
				.as_bytes(),
		)
	};
	let dir = root.join("infra/notes-production");
	fs::create_dir_all(&dir).unwrap();
	fs::write(root.join("reviewer.key"), hex::encode([1_u8; 32])).unwrap();
	fs::write(root.join("deployment.key"), hex::encode([2_u8; 32])).unwrap();
	succeeds(&cli(
		root,
		&[
			"verify",
			"approve-backend",
			"--descriptor",
			descriptor,
			"--key",
			"reviewer.key",
			"--out",
			"approval.json",
		],
		&[],
	));
	let approval: serde_json::Value =
		serde_json::from_slice(&fs::read(root.join("approval.json")).unwrap()).unwrap();
	let bytes = fs::read_to_string(descriptor).unwrap();
	let parsed: serde_json::Value = serde_json::from_str(&bytes).unwrap();
	let trust = serde_json::json!({"format_version":1,"repository":"https://github.com/example/backend","deployment":"production","publisher":public(3),"approvers":[public(1)],"threshold":1,"deployment_keys":[public(2)]});
	fs::write(
		dir.join("backend-trust.json"),
		serde_json::to_vec(&trust).unwrap(),
	)
	.unwrap();
	let evidence = serde_json::json!({"format_version":1,"descriptor":bytes,"approvals":[approval],"deployment_policy":""});
	fs::write(
		dir.join("backend-authorization.json"),
		serde_json::to_vec(&evidence).unwrap(),
	)
	.unwrap();
	let now = chrono::Utc::now().timestamp();
	let policy = serde_json::json!({"format_version":2,"kind":"leptoskit-deployment-policy","repository":"https://github.com/example/backend","deployment":"production","origins":["https://app.example.com"],"sequence":1,"issued_at":now-60,"expires_at":now+3600,"releases":[{"commit":parsed["recipe"]["commit"],"inventory_sha256":parsed["inventory_sha256"],"executable_sha256":parsed["executable_sha256"],"backend":{"descriptor_sha256":pin,"engine_image_id":parsed["engine_image_id"]}}]});
	fs::write(
		root.join("policy.json"),
		serde_json::to_vec(&policy).unwrap(),
	)
	.unwrap();
	succeeds(&cli(
		root,
		&[
			"verify",
			"sign",
			"--policy",
			"policy.json",
			"--key",
			"deployment.key",
			"--out",
			"infra/notes-production/backend-deployment.json",
		],
		&[],
	));
}

#[test]
fn backend_approvals_gate_release_reuse_rollback_and_recovery() {
	let root = project();
	let (path, pin) = verified_descriptor(root.path());
	backend_authorization(root.path(), &path, &pin);
	fails_with(
		&cli(
			root.path(),
			&["deploy", "--setup", "--version", "unsigned"],
			&[],
		),
		"prohibit unsigned/legacy releases",
	);
	let dir = root.path().join("infra/notes-production");
	let flags = [
		"deploy",
		"--setup",
		"--version",
		"v1",
		"--verified-descriptor",
		&path,
		"--descriptor-sha256",
		&pin,
	];
	succeeds(&cli(root.path(), &flags, &[]));
	let policy = fs::read(dir.join("backend-deployment.json")).unwrap();
	for flags in [
		vec!["deploy", "--rollback", "v1"],
		vec!["deploy", "--recover", "--version", "v1"],
	] {
		fs::write(dir.join("backend-deployment.json"), "{}").unwrap();
		fs::write(dir.join("kamal-calls"), "").unwrap();
		fails_with(
			&cli(root.path(), &flags, &[]),
			"invalid signed deployment policy",
		);
		let calls = fs::read_to_string(dir.join("kamal-calls")).unwrap();
		assert!(!calls.contains("app boot") && !calls.contains("proxy boot"));
		fs::write(dir.join("backend-deployment.json"), &policy).unwrap();
		succeeds(&cli(root.path(), &flags, &[]));
	}

	let history_path = root
		.path()
		.join("remote/state/notes-production/control/releases.json");
	let mut history: serde_json::Value =
		serde_json::from_slice(&fs::read(&history_path).unwrap()).unwrap();
	history.as_array_mut().unwrap().last_mut().unwrap()["authorization"]["approvals"][0]
		["signature"] = serde_json::json!("00".repeat(64));
	fs::write(&history_path, serde_json::to_vec(&history).unwrap()).unwrap();
	fs::write(root.path().join("remote/ssh-calls"), "").unwrap();
	fails_with(
		&cli(
			root.path(),
			&[
				"deploy",
				"--version",
				"v2",
				"--verified-descriptor",
				&path,
				"--descriptor-sha256",
				&pin,
			],
			&[("MOCK_KAMAL_FAIL_VERSION", "v2")],
		),
		"Automatic recovery did not complete",
	);
	let calls = fs::read_to_string(root.path().join("remote/ssh-calls")).unwrap();
	assert!(calls.contains("docker stop"));
	fs::remove_file(dir.join("backend-trust.json")).unwrap();
	fails_with(
		&cli(
			root.path(),
			&["deploy", "--recover", "--version", "v1"],
			&[],
		),
		"requires operator backend-trust.json",
	);
}
