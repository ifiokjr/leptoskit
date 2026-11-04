#!/usr/bin/env python3
"""Validate the pilot's real OpenTofu configuration without cloud credentials."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def check(cli, tofu):
    repository = Path(__file__).resolve().parents[2]
    template = repository / "crates/leptoskit_app/deploy/pilot/leptoskit.toml.example"
    text = template.read_text().replace("ssh_key_ids = []", "ssh_key_ids = [123]")
    text = text.replace("ssh_cidrs = []", 'ssh_cidrs = ["192.0.2.1/32"]')
    # A validation fixture, not approval to spend or provision these resources.
    text = text.replace("monthly_compute = 0", "monthly_compute = 6")
    environment = dict(os.environ)
    for name in list(environment):
        if name.startswith(("TF_", "DIGITALOCEAN_", "CLOUDFLARE_", "STRIPE_")):
            environment.pop(name)
    with tempfile.TemporaryDirectory(prefix="leptoskit-pilot-infra-") as temporary:
        root = Path(temporary)
        (root / "leptoskit.toml").write_text(text)
        subprocess.run([str(cli), "infra", "generate"], cwd=root, env=environment,
                       check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
        if (root / "Dockerfile").exists():
            raise RuntimeError("external provisioning unexpectedly generated a container recipe")
        source = root / "infra/leptoskit-pilot"
        value = json.loads((source / "main.tf.json").read_text())
        if "hcloud_server" in value["resource"] or "cloudflare_r2_bucket" in value["resource"]:
            raise RuntimeError("the minimal DigitalOcean pilot contains unexpected resources")
        for arguments in (["init", "-backend=false", "-input=false", "-no-color"],
                          ["validate", "-no-color"]):
            result = subprocess.run([str(tofu), *arguments], cwd=source, env=environment,
                                    capture_output=True, text=True, timeout=300)
            if result.returncode:
                raise RuntimeError(result.stdout + result.stderr)
        version = subprocess.check_output([str(tofu), "version", "-json"], cwd=source,
                                          env=environment, text=True)
        return {"opentofu_configuration": "validated", "compute": "digitalocean",
                "toolchain": json.loads(version), "cloud_apply": "not run"}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True)
    parser.add_argument("--tofu", required=True)
    arguments = parser.parse_args()
    cli = shutil.which(arguments.cli)
    tofu = shutil.which(arguments.tofu)
    if not cli or not tofu:
        raise SystemExit("provide installed LeptosKit and OpenTofu executable paths")
    print(json.dumps(check(Path(cli).resolve(), Path(tofu).resolve()), indent=2))
