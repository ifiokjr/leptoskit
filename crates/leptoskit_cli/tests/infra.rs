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
init) printf lock > .terraform.lock.hcl ;;
plan) printf plan > leptoskit.plan ;;
show)
  if [ -n "${MOCK_PLAN_JSON:-}" ]; then printf '%s' "$MOCK_PLAN_JSON";
  else printf '%s' '{"format_version":"1.2","resource_changes":[]}'; fi ;;
output)
  if [ "$#" -eq 2 ]; then printf '{"server_ip":{"value":"192.0.2.10"}}'; exit 0; fi
  case "$3" in
  server_ip) printf '192.0.2.10' ;;
  server_ips) if [ -n "${MOCK_HOSTS:-}" ]; then printf '%s' "$MOCK_HOSTS"; else printf '["192.0.2.10"]'; fi ;;
  private_ip) printf '10.42.0.10' ;;
  backup_endpoint) printf 'https://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.r2.cloudflarestorage.com' ;;
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
if os.environ.get("MOCK_SSH_EXIT"):
    sys.exit(1)
if "pg_dump" in script:
    print("PGDMPfixture", end="")
elif "io.leptoskit.migrations" in script:
    print(os.environ["MOCK_SCHEMA_LABEL"])
elif "json .RepoDigests" in script:
    digest = os.environ.get("MOCK_SECONDARY_DIGEST") if target.endswith("192.0.2.11") else None
    print(json.dumps(["ghcr.io/team/notes@sha256:" + (digest or os.environ.get("MOCK_DIGEST", "a" * 64))]))
elif script.startswith("docker exec") and "_leptoskit_migrations" in script:
    print(os.environ.get("MOCK_HISTORY", "[]"))
elif script.startswith("python3 "):
    print(json.dumps({"healthy": True, "verified": True, "snapshot": "a" * 64}))
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
