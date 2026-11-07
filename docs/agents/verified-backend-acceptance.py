"""Real Podman acceptance. Run after committing with the CLI path as argument.

Uses a signed commit of the committed fixture tree and invocation-only local
Git transport mapping. Tests full reproduction mechanics, not public provenance.
No checkout, registry publication, deployment, or shared Git config changes.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def run(*args, env=None, success=True):
    result = subprocess.run(args, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    print(result.stdout, flush=True)
    assert (result.returncode == 0) == success, args
    return result.stdout.strip()


def digest(data):
    return hashlib.sha256(data).hexdigest()


root = Path(__file__).resolve().parents[2]
cli = str(Path(sys.argv[1]).resolve())
tree = run("git", "rev-parse", "HEAD:crates/leptoskit_cli/tests/fixtures/verified-backend")
commit = run("git", "commit-tree", "-S", tree, "-m", "test: committed backend acceptance fixture")
env = dict(os.environ, GIT_CONFIG_COUNT="1", GIT_CONFIG_KEY_0=f"url.file://{root}.insteadOf", GIT_CONFIG_VALUE_0="https://github.com/leptoskit/leptoskit")
images = json.loads(run("podman", "image", "inspect", "rust:1.85-alpine", "alpine:3.21"))
builder, runtime = [item["RepoDigests"][0] for item in images]
runtime_pin = digest(json.dumps(images[1]["Config"], sort_keys=True, separators=(",", ":")).encode())
print(json.dumps(dict(commit=commit, builder=builder, runtime=runtime, runtime_config=runtime_pin)))
with tempfile.TemporaryDirectory(prefix="a04-acceptance-") as temp:
    temp = Path(temp)
    recipe = temp / "recipe.json"
    recipe.write_text(json.dumps(dict(repository="https://github.com/leptoskit/leptoskit", commit=commit, builder_image=builder, target="aarch64-unknown-linux-musl", package="verified-backend", binary="verified-backend", features=[], no_default_features=False, rustflags=[])))
    out = temp / "build"
    run(cli, "verify", "build", "--recipe", str(recipe), "--out", str(out), "--engine", "podman", env=env)
    pin = digest((out / "inventory.json").read_bytes())
    common = ["--bundle", str(out / "bundle"), "--manifest", str(out / "inventory.json"), "--sha256", pin, "--engine", "podman"]
    run(cli, "verify", "reproduce", *common, env=env)
    tag = f"localhost/a04-acceptance:{os.getpid()}"
    descriptor = temp / "descriptor.json"
    package = [cli, "infra", "build-verified", *common, "--runtime-image", runtime, "--runtime-config-sha256", runtime_pin, "--tag", tag]
    try:
        run(*package, "--out", str(descriptor), env=env)
        print(descriptor.read_text())
        run(*package, "--reuse-existing", "--out", str(temp / "reuse.json"), env=env)
        for relative in ["artifacts/backend", "source/src/main.rs", "source/migrations/registry.txt", "source/leptoskit.toml", "build.json"]:
            path = out / "bundle" / relative
            original = path.read_bytes()
            path.write_bytes(original + b"substitution")
            failed = temp / "failed.json"
            run(*package, "--out", str(failed), env=env, success=False)
            assert not failed.exists(), "failure wrote success evidence"
            path.write_bytes(original)
        failed = temp / "failed-config.json"
        changed = package.copy()
        changed[changed.index("--runtime-config-sha256") + 1] = "0" * 64
        run(*changed, "--reuse-existing", "--out", str(failed), env=env, success=False)
        assert not failed.exists()
        # Preserve the schema label and entrypoint but replace the actual bytes.
        wrong = temp / "wrong"
        wrong.mkdir()
        (wrong / "backend").write_bytes(b"wrong executable")
        (wrong / "Dockerfile").write_text(f"FROM {tag}\nCOPY --chmod=0755 backend /app/backend\n")
        run("podman", "build", "--network=none", "--tag", tag, str(wrong))
        failed = temp / "failed-bytes.json"
        run(*package, "--reuse-existing", "--out", str(failed), env=env, success=False)
        assert not failed.exists()
        print("REAL ENGINE ACCEPTANCE PASSED; local fixture transport, no public provenance claim")
    finally:
        run("podman", "image", "rm", "--force", tag)
