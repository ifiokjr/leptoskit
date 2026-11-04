#!/usr/bin/env bash
# Build on Linux; the resulting archive contains no credentials or state.
set -euo pipefail
umask 022

if [[ $# != 2 ]]; then
	echo "usage: $0 HTTPS_ORIGIN OUTPUT_DIRECTORY" >&2
	exit 2
fi
pilot_origin="$1"
pilot_output="$(python3 -c 'import os,sys; print(os.path.abspath(sys.argv[1]))' "$2")"
python3 - "$pilot_origin" <<'PY'
import sys
from urllib.parse import urlsplit
url = urlsplit(sys.argv[1])
if (url.scheme != 'https' or not url.hostname or url.username or url.password
        or url.path not in ('', '/') or url.query or url.fragment
        or url.port not in (None, 443)):
    raise SystemExit('use a public HTTPS origin without credentials, path, query or fragment')
PY
if [[ "$(uname -s)" != Linux ]]; then
	echo "Build the hosted pilot on Linux so its broker runs on the deployment host." >&2
	exit 2
fi
if [[ -e "$pilot_output" || -L "$pilot_output" || -e "$pilot_output.tar.gz" || -L "$pilot_output.tar.gz" ]]; then
	echo "Choose a new output directory; existing releases are never overwritten." >&2
	exit 2
fi

pilot_root="$(git rev-parse --show-toplevel)"
cd "$pilot_root"
if [[ -n "$(git status --porcelain)" ]]; then
	echo "Commit the candidate first so release.json identifies the complete source." >&2
	exit 2
fi
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
pilot_target="$(rustc -vV | sed -n 's/^host: //p')"
pilot_target_dir="$(cargo metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
pilot_revision="$(git rev-parse HEAD)"
mkdir -p "$(dirname "$pilot_output")"
pilot_stage="$(mktemp -d "${pilot_output}.tmp.XXXXXX")"
trap 'rm -rf "$pilot_stage"' EXIT
chmod 0755 "$pilot_stage"

cargo build --locked --release --target "$pilot_target" -p leptoskit_cli --bin leptoskit
cargo build --locked --release --target "$pilot_target" -p leptoskit_app --features site --bins
"$pilot_target_dir/$pilot_target/release/leptoskit-site" \
	--url "$pilot_origin" \
	--cli "$pilot_target_dir/$pilot_target/release/leptoskit" \
	--out "$pilot_stage/site" --release
install -m 0755 "$pilot_target_dir/$pilot_target/release/leptoskit-cloud" "$pilot_stage/leptoskit-cloud"
mkdir "$pilot_stage/systemd"
install -m 0644 crates/leptoskit_app/deploy/leptoskit-cloud.service "$pilot_stage/systemd/"
install -m 0644 crates/leptoskit_app/deploy/leptoskit-cloud.sysusers "$pilot_stage/systemd/"
install -m 0644 crates/leptoskit_app/deploy/broker.env.example "$pilot_stage/"
python3 - "$pilot_stage" "$pilot_revision" "$pilot_origin" "$pilot_target" <<'PY'
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import sys
root = Path(sys.argv[1])
binary = root / 'leptoskit-cloud'
with binary.open('rb') as source:
    if source.read(4) != b'\x7fELF':
        raise SystemExit('the deployment broker must be a Linux ELF executable')
linkage = subprocess.check_output(['readelf', '-l', '-d', str(binary)], text=True)
if '/nix/store/' in linkage:
    raise SystemExit('the broker requires the Nix store; build with the system Linux compiler')
operating_system = platform.freedesktop_os_release()
(root / 'release.json').write_text(json.dumps({
    'revision': sys.argv[2], 'origin': sys.argv[3].rstrip('/'),
    'target': sys.argv[4], 'build_os': {
        'id': operating_system.get('ID'), 'version': operating_system.get('VERSION_ID'),
    },
}, indent=2) + '\n')
files = sorted(path for path in root.rglob('*') if path.is_file())
with (root / 'SHA256SUMS').open('w') as manifest:
    for path in files:
        with path.open('rb') as source:
            digest = hashlib.file_digest(source, 'sha256').hexdigest()
        manifest.write(f'{digest}  {path.relative_to(root).as_posix()}\n')
PY
mv "$pilot_stage" "$pilot_output"
tar -C "$pilot_output" -czf "$pilot_output.tar.gz" .
echo "Hosted pilot bundle: $pilot_output.tar.gz"
