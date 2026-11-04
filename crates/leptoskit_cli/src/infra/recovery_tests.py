"""Exercise real PostgreSQL and Restic with a local Docker transport adapter.

The adapter substitutes separate databases for containers; archive encryption,
dump/restore, SQL, ownership, and atomic database replacement are real.
"""
import json
import os
import pathlib
import subprocess
import sys

worker, database = sys.argv[1:]
root = pathlib.Path(worker).parent
bin_path = root / "bin"
bin_path.mkdir()
os.environ["PATH"] = str(bin_path) + os.pathsep + os.environ["PATH"]
os.environ["LK_TEST_ROOT"] = str(root)


def psql(query, db=database):
    return subprocess.check_output(["psql", db, "-v", "ON_ERROR_STOP=1", "-At", "-c", query]).decode().strip()


# The fixture's trust-only listener is local and ephemeral.
from urllib.parse import urlparse
url = urlparse(database)
os.environ.update(PGHOST=url.hostname, PGPORT=str(url.port), PGUSER=url.username or "postgres")
psql("CREATE ROLE leptoskit_admin SUPERUSER LOGIN; CREATE ROLE leptoskit LOGIN")
psql("CREATE DATABASE leptoskit OWNER leptoskit")
psql("CREATE TABLE _leptoskit_migrations(name TEXT PRIMARY KEY, checksum TEXT NOT NULL); "
     "INSERT INTO _leptoskit_migrations VALUES ('first', 'fixture'); "
     "CREATE TABLE notes(body TEXT); INSERT INTO notes VALUES ('original'); "
     "ALTER TABLE notes OWNER TO leptoskit; ALTER TABLE _leptoskit_migrations OWNER TO leptoskit", "leptoskit")

adapter = bin_path / "docker"
adapter.write_text(r'''#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys
root = pathlib.Path(os.environ["LK_TEST_ROOT"])
state = root / "containers.json"
mapping = json.loads(state.read_text()) if state.exists() else {}
args = sys.argv[1:]
def sql(query):
    subprocess.run(["psql", "-d", "postgres", "-v", "ON_ERROR_STOP=1", "-c", query], stdout=subprocess.DEVNULL, check=True)
if args[0] == "run":
    container = args[args.index("--name") + 1]
    name = "verify_" + container.replace("-", "_")[-24:]
    sql("CREATE DATABASE " + name)
    mapping[container] = name
    state.write_text(json.dumps(mapping))
    print(container)
elif args[0] == "rm":
    container = args[-1]
    if container in mapping:
        sql("DROP DATABASE " + mapping.pop(container))
        state.write_text(json.dumps(mapping))
elif args[0] == "exec":
    args.pop(0)
    if args[0] == "-i": args.pop(0)
    container = args.pop(0)
    if "FLUSHALL" in " ".join(args):
        (root / "cache-stale").unlink(missing_ok=True)
        sys.exit(0)
    if "-d" in args and args[args.index("-d") + 1] == "verify":
        args[args.index("-d") + 1] = mapping[container]
    if os.environ.get("LK_FAIL_RESTORE") and args[0] == "pg_restore": sys.exit(1)
    sys.exit(subprocess.run(args).returncode)
elif args[0] in ["stop", "start"]:
    pass
else:
    sys.exit(99)
''')
adapter.chmod(0o700)
(root / "recovery.json").write_text(json.dumps({"name": "fixture", "services": {"relay": True, "cache": True}, "repository": str(root / "repo"),
    "daily": 7, "weekly": 4, "monthly": 3, "max_age_hours": 36}))
(root / "backup-secrets.json").write_text(json.dumps({"RESTIC_PASSWORD": "fixture-encryption-password"}))
(root / "releases.json").write_text('[{"version":"v1"}]')
(root / "cache-stale").write_text("stale value")
(root.parent / "nats").mkdir(mode=0o700)
(root.parent / "nats/old-stream").write_text("events ahead of the restored database")


def invoke(*args, success=True):
    result = subprocess.run([sys.executable, worker, *args], capture_output=True)
    assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
    return json.loads(result.stdout)


invoke("init")
assert invoke("status")["healthy"] is False
snapshot = invoke("backup")["snapshot"]
assert invoke("status")["healthy"] is True
assert len(invoke("snapshots")) == 1
invoke("verify", "--snapshot", snapshot)
psql("UPDATE notes SET body = 'changed'", "leptoskit")
invoke("restore", "--snapshot", snapshot, success=False)
assert psql("SELECT body FROM notes", "leptoskit") == "changed"
restored = invoke("restore", "--snapshot", snapshot, "--yes")
assert restored["application_stopped"]
assert psql("SELECT body FROM notes", "leptoskit") == "original"
assert psql("SELECT body FROM notes", restored["previous_database"]) == "changed"
assert json.loads((root / "releases.json").read_text()) == [{"version": "v1"}]
assert not (root / "cache-stale").exists()
assert not list((root.parent / "nats").iterdir())
assert len(list(root.parent.glob("nats-before-restore-*/old-stream"))) == 1
assert not (root / "derived-reset-required.json").exists()
assert not (root / "operation.lock").exists()

# Restore verification failure blocks pruning and leaves the original intact.
os.environ["LK_FAIL_RESTORE"] = "1"
invoke("backup", success=False)
assert invoke("status")["healthy"] is False
assert psql("SELECT body FROM notes", "leptoskit") == "original"
assert len(invoke("snapshots")) == 2
assert not (root / "snapshot").exists()
del os.environ["LK_FAIL_RESTORE"]

# No implicit expiry or unlock can steal another operation's reservation.
owner = "163e4567-e89b-42d3-a456-426614174000"
(root / "operation.lock").write_text(owner)
invoke("backup", success=False)
assert (root / "operation.lock").read_text() == owner
(root / "operation.lock").unlink()

# An incorrect key cannot read an archive or replace the database.
(root / "backup-secrets.json").write_text('{"RESTIC_PASSWORD":"wrong-password"}')
invoke("verify", "--snapshot", snapshot, success=False)
assert psql("SELECT body FROM notes", "leptoskit") == "original"
print("Verified encrypted backup, restore, ownership, retention gating, and reservations")
