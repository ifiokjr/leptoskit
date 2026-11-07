#!/usr/bin/env bash
# Writes the project the services deploy checks run: this checkout's tracked
# files, with examples/analytics promoted to the project root, where a
# `leptoskit.toml` of its own adds the echo service from setup/services_echo.
#
#   services_deploy_fixture.sh <leptoskit> <destination> <sqlite|postgres> <echo trust>
#
# Then it writes the deployment files with `<leptoskit> generate deploy` for
# Compose and Kubernetes. The project is a copy of the whole workspace
# because the example's crates take their dependencies from it, and the
# generated Dockerfile builds whatever is beside `leptoskit.toml`.
#
# The one change to a generated file: the Dockerfile installs the leptoskit
# CLI from crates.io, which has no release with this checkout's changes, so
# its build stage copies in the CLI built from this checkout instead, from
# the build context `leptoskit-cli` (`docker build --build-context
# leptoskit-cli=<directory with leptoskit>`).
set -euo pipefail

if [ "$#" -ne 4 ]; then
	echo "usage: $0 <leptoskit> <destination> <sqlite|postgres> <isolated|first-party>" >&2
	exit 2
fi

cli="$(realpath "$1")"
destination="$2"
dialect="$3"
echo_trust="$4"
repository="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"

case "$dialect" in
sqlite | postgres) ;;
*)
	echo "the dialect is sqlite or postgres, not $dialect" >&2
	exit 2
	;;
esac

if [ -e "$destination" ]; then
	echo "$destination already exists" >&2
	exit 1
fi

mkdir -p "$destination"
# Files git knows or would add, so nothing local (targets, databases)
# reaches the image.
git -C "$repository" ls-files -z --cached --others --exclude-standard | (cd "$repository" && tar --null -T - -cf -) | tar -xf - -C "$destination"
cp -R "$repository/examples/analytics/analytics_app/public" "$destination/public"

# The toolchain the checkout pins, without the components only development
# needs.
channel="$(sed -n 's/^channel = "\(.*\)"$/\1/p' "$repository/rust-toolchain.toml")"
cat >"$destination/rust-toolchain.toml" <<EOF
[toolchain]
channel = "$channel"
profile = "minimal"
targets = ["wasm32-unknown-unknown"]
EOF

{
	cat <<'EOF'
#:schema https://leptoskit.dev/schemas/leptoskit.v0.schema.json
# examples/analytics, deployed by .github/workflows/services-deploy.yml.
schema_version = 0

[app]
name = "analytics_app"
render = "ssr"

[serve]
address = "127.0.0.1"
port = 3000
EOF
	if [ "$dialect" = postgres ]; then
		cat <<'EOF'

# The app's database is Postgres, so each service gets a database and a role
# of its own on the same server.
[db]
dialect = "postgres"

[services.analytics]
package = "analytics_service"
features = ["service", "postgres"]
EOF
	else
		cat <<'EOF'

[services.analytics]
package = "analytics_service"
EOF
	fi
	cat <<EOF

# Reports the cookies that reach it, so the checks see what the edge strips.
[services.echo]
package = "services_echo"
trust = "$echo_trust"
EOF
} >"$destination/leptoskit.toml"

(
	cd "$destination"
	"$cli" generate deploy --target compose
	if [ "$echo_trust" = first-party ]; then
		"$cli" generate deploy --target kubernetes
	fi
)

# shellcheck disable=SC2016 # the Dockerfile's own variable, kept as written
install_line='RUN cargo install leptoskit_cli --version "${LEPTOSKIT_VERSION}" --locked'
if ! grep -qxF "$install_line" "$destination/Dockerfile"; then
	echo "the generated Dockerfile no longer installs the CLI with: $install_line" >&2
	exit 1
fi
replacement='COPY --from=leptoskit-cli leptoskit /usr/local/cargo/bin/leptoskit'
python3 - "$destination/Dockerfile" "$install_line" "$replacement" <<'EOF'
import sys
path, old, new = sys.argv[1:]
text = open(path).read()
open(path, "w").write(text.replace(old + "\n", new + "\n", 1))
EOF
