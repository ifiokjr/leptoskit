#!/usr/bin/env python3
"""Private host-side recovery worker, generated and configured by the Rust CLI."""
import argparse
import contextlib
import datetime
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

ROOT = pathlib.Path(__file__).resolve().parent
CONFIG = json.loads((ROOT / "recovery.json").read_text())
SECRETS = json.loads((ROOT / "backup-secrets.json").read_text())
os.environ.update(SECRETS)
os.environ["RESTIC_REPOSITORY"] = CONFIG["repository"]
os.environ["RESTIC_CACHE_DIR"] = str(ROOT / "restic-cache")
TAG = "leptoskit-" + CONFIG["name"]
STAGE = ROOT / "snapshot"
DB = CONFIG["name"] + "-db"
MIGRATIONS = "SELECT COALESCE(json_agg(json_build_array(name, checksum)), '[]'::json) FROM _leptoskit_migrations"


def run(args, stdin=None, stdout=subprocess.PIPE):
    result = subprocess.run(args, stdin=stdin, stdout=stdout, stderr=subprocess.PIPE, check=False)
    if result.returncode:
        # Command arguments and stderr can contain credentials. Journal only a label.
        raise RuntimeError(args[0] + " failed with exit " + str(result.returncode))
    return result.stdout


def atomic(path, value):
    fd, temporary = tempfile.mkstemp(dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(value, stream)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        pathlib.Path(temporary).unlink(missing_ok=True)


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


@contextlib.contextmanager
def reservation(existing):
    path = ROOT / "operation.lock"
    owner = existing or str(uuid.uuid4())
    if existing:
        if path.read_text() != owner:
            raise RuntimeError("environment reservation changed")
    else:
        candidate = ROOT / (owner + ".owner")
        candidate.write_text(owner)
        try:
            os.link(candidate, path)
        finally:
            candidate.unlink(missing_ok=True)
    try:
        yield
        if path.read_text() != owner:
            raise RuntimeError("environment reservation changed")
    finally:
        if not existing and path.exists() and path.read_text() == owner:
            path.unlink()


def sql(container, database, query, user="postgres"):
    return run(["docker", "exec", container, "psql", "-U", user, "-d", database, "-v", "ON_ERROR_STOP=1", "-At", "-c", query])


def schema(container, database, user="postgres"):
    return sorted(json.loads(sql(container, database, MIGRATIONS, user)))


def archive(snapshot, filename, destination):
    if snapshot != "latest" and not re.fullmatch(r"[a-f0-9]{8,64}", snapshot):
        raise RuntimeError("invalid snapshot id")
    with destination.open("wb") as stream:
        run(["restic", "dump", snapshot, str(STAGE / filename), "--tag", TAG], stdout=stream)


def restore_archive(container, database, path, user="postgres", role=None):
    args = ["docker", "exec", "-i", container, "pg_restore", "-U", user, "-d", database,
            "--exit-on-error", "--no-owner", "--no-acl"]
    if role:
        args += ["--role", role]
    with path.open("rb") as stream:
        run(args, stdin=stream)


def verify(snapshot):
    with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
        temporary = pathlib.Path(temporary)
        archive(snapshot, "database.dump", temporary / "database.dump")
        archive(snapshot, "metadata.json", temporary / "metadata.json")
        metadata = json.loads((temporary / "metadata.json").read_text())
        container = "lk-verify-" + uuid.uuid4().hex
        try:
            run(["docker", "run", "-d", "--name", container, "--network", "none",
                 "-e", "POSTGRES_HOST_AUTH_METHOD=trust", "-e", "POSTGRES_DB=verify", "postgres:17"])
            for attempt in range(60):
                ready = subprocess.run(["docker", "exec", container, "pg_isready", "-U", "postgres"],
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                if ready.returncode == 0:
                    break
                time.sleep(1)
            else:
                raise RuntimeError("verification database did not become ready")
            restore_archive(container, "verify", temporary / "database.dump")
            if schema(container, "verify") != metadata["migrations"]:
                raise RuntimeError("restored migration history differs from backup metadata")
        finally:
            subprocess.run(["docker", "rm", "-f", "-v", container], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return metadata


def snapshots():
    return json.loads(run(["restic", "snapshots", "--json", "--tag", TAG]))


def resolve(snapshot):
    available = snapshots()
    matching = available if snapshot == "latest" else [item for item in available if item["id"].startswith(snapshot)]
    if not matching or (snapshot != "latest" and len(matching) != 1):
        raise RuntimeError("snapshot is absent or ambiguous in this deployment's repository")
    return max(matching, key=lambda item: item["time"])["id"]


def backup():
    shutil.rmtree(STAGE, ignore_errors=True)
    STAGE.mkdir(mode=0o700)
    try:
        metadata = {"created_at": now(), "migrations": schema(DB, "leptoskit", "leptoskit")}
        with (STAGE / "database.dump").open("wb") as stream:
            run(["docker", "exec", DB, "pg_dump", "-U", "leptoskit", "-d", "leptoskit", "--format=custom"], stdout=stream)
        atomic(STAGE / "metadata.json", metadata)
        (STAGE / "releases.json").write_bytes((ROOT / "releases.json").read_bytes() if (ROOT / "releases.json").exists() else b"[]")
        output = run(["restic", "backup", str(STAGE), "--json", "--tag", TAG, "--host", CONFIG["name"]])
        summaries = [json.loads(line) for line in output.splitlines() if line.strip()]
        snapshot = next(item["snapshot_id"] for item in reversed(summaries) if item.get("message_type") == "summary")
        verify(snapshot)
        atomic(ROOT / "backup-status.json", {"snapshot": snapshot, "completed_at": now(), "verified": True})
        retention()
        return {"snapshot": snapshot, "verified": True}
    finally:
        shutil.rmtree(STAGE, ignore_errors=True)


def retention():
    # Pruning is only reached after a completed restore check. Each deployment
    # has a separate encrypted repository prefix as well as a snapshot tag.
    run(["restic", "forget", "--tag", TAG, "--keep-daily", str(CONFIG["daily"]),
         "--keep-weekly", str(CONFIG["weekly"]), "--keep-monthly", str(CONFIG["monthly"]), "--prune"])
    run(["restic", "check"])


def reset_derived_state(suffix):
    services = CONFIG.get("services", {})
    if services.get("relay"):
        # A restored change log can be behind JetStream. Start a fresh derived
        # stream; publishers repopulate it from Postgres and clients resubscribe.
        container = CONFIG["name"] + "-relay"
        run(["docker", "stop", "--time", "30", container])
        path = ROOT.parent / "nats"
        owner = path.stat()
        path.rename(ROOT.parent / ("nats-before-restore-" + suffix))
        path.mkdir(mode=0o700)
        os.chown(path, owner.st_uid, owner.st_gid)
        run(["docker", "start", container])
    if services.get("cache"):
        run(["docker", "exec", CONFIG["name"] + "-cache", "sh", "-ec",
             'VALKEYCLI_AUTH="$VALKEY_PASSWORD" valkey-cli FLUSHALL'])


def restore(snapshot, confirmed):
    if not confirmed:
        raise RuntimeError("restore requires explicit --yes and stopped application writers")
    verify(snapshot)
    suffix = uuid.uuid4().hex[:16]
    candidate, previous = "lk_restore_" + suffix, "lk_previous_" + suffix
    with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
        temporary = pathlib.Path(temporary)
        archive(snapshot, "database.dump", temporary / "database.dump")
        archive(snapshot, "metadata.json", temporary / "metadata.json")
        archive(snapshot, "releases.json", temporary / "releases.json")
        metadata = json.loads((temporary / "metadata.json").read_text())
        history = json.loads((temporary / "releases.json").read_text())
        active = sql(DB, "postgres", "SELECT count(*) FROM pg_stat_activity WHERE datname = 'leptoskit'", "leptoskit_admin")
        if int(active) != 0:
            raise RuntimeError("database still has clients; stop all writers before restore")
        sql(DB, "postgres", "CREATE DATABASE " + candidate + " OWNER leptoskit", "leptoskit_admin")
        restore_archive(DB, candidate, temporary / "database.dump", "leptoskit_admin", "leptoskit")
        if schema(DB, candidate, "leptoskit_admin") != metadata["migrations"]:
            raise RuntimeError("candidate schema verification failed; original database is unchanged")
        # Both renames commit together. PostgreSQL rejects the rename if a
        # racing unmanaged client connected; it does not kill client sessions.
        atomic(ROOT / "derived-reset-required.json", {"snapshot": snapshot})
        sql(DB, "postgres", "BEGIN; ALTER DATABASE leptoskit RENAME TO " + previous +
            "; ALTER DATABASE " + candidate + " RENAME TO leptoskit; COMMIT", "leptoskit_admin")
        atomic(ROOT / "releases.json", history)
        reset_derived_state(suffix)
        (ROOT / "derived-reset-required.json").unlink()
        result = {"restored_at": now(), "snapshot": snapshot, "previous_database": previous,
                  "migrations": metadata["migrations"], "application_stopped": True}
        atomic(ROOT / "restore-status.json", result)
        return result


def status():
    path = ROOT / "backup-status.json"
    result = json.loads(path.read_text()) if path.exists() else {"verified": False}
    completed = result.get("completed_at")
    age = (datetime.datetime.now(datetime.timezone.utc) - datetime.datetime.fromisoformat(completed)).total_seconds() if completed else None
    result["healthy"] = bool(result.get("verified") and age is not None and age <= CONFIG["max_age_hours"] * 3600)
    result["age_seconds"] = age
    failure = ROOT / "backup-error.json"
    if failure.exists():
        result["last_error"] = json.loads(failure.read_text())
        result["healthy"] = False
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=["init", "backup", "verify", "snapshots", "restore", "status"])
    parser.add_argument("--snapshot", default="latest")
    parser.add_argument("--reservation")
    parser.add_argument("--yes", action="store_true")
    args = parser.parse_args()
    os.umask(0o077)
    if args.command == "status":
        result = status()
    else:
        with reservation(args.reservation):
            if args.command == "init":
                exists = subprocess.run(["restic", "cat", "config"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                if exists.returncode:
                    run(["restic", "init"])
                result = {"initialized": True}
            elif args.command == "snapshots":
                result = snapshots()
            elif args.command == "verify":
                result = verify(resolve(args.snapshot))
            elif args.command == "restore":
                result = restore(resolve(args.snapshot), args.yes)
            else:
                try:
                    result = backup()
                    (ROOT / "backup-error.json").unlink(missing_ok=True)
                except Exception:
                    atomic(ROOT / "backup-error.json", {"failed_at": now(), "error": "backup, verification, or retention failed; inspect private operator logs"})
                    raise
    print(json.dumps(result))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(json.dumps({"error": str(error)}))
        sys.exit(1)
