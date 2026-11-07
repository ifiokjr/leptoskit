#!/usr/bin/env bash
# Loads the Kamal configuration `lk infra` releases with into a real Kamal,
# and checks the commands Kamal derives from it.
#
#   services_deploy_kamal.sh
#
# The configurations are the insta snapshots `leptoskit_cli`'s infra tests
# pin, so what Kamal reads here is exactly what `runtime::kamal` writes:
# `managed_kamal` (no services) and `managed_services` (two process services,
# one public, and a remote one). `kamal config` parses and validates each one
# without a host; the Ruby below asks Kamal for the `docker run` and
# kamal-proxy options of each role, which is where the §8 claims of
# docs/agents/services.md land.
set -euo pipefail

repository="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
snapshots="$repository/crates/leptoskit_cli/src/infra/snapshots"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

fail() {
	echo "FAIL $*" >&2
	exit 1
}

pass() {
	echo "PASS $*"
}

# An insta snapshot's body: everything after its front matter.
body() {
	awk 'found { print } /^---$/ { seen++; if (seen == 2) found = 1 }' "$1"
}

kamal version

# Each configuration, with the snapshot of the secrets file `lk infra`
# writes beside it.
for pair in managed_kamal:managed_secret_references managed_services:managed_services_secret_references; do
	name="${pair%%:*}"
	secrets="${pair#*:}"
	dir="$work/$name"
	mkdir -p "$dir/.kamal"
	body "$snapshots/leptoskit_cli__infra__tests__$name.snap" >"$dir/release.yml"
	body "$snapshots/leptoskit_cli__infra__tests__$secrets.snap" >"$dir/.kamal/secrets"
	[ -s "$dir/release.yml" ] && [ -s "$dir/.kamal/secrets" ] || fail "$name: empty snapshot"
	grep -q '^{' "$dir/release.yml" || fail "$name: release.yml isn't the configuration"
	grep -q '^[A-Z_][A-Z0-9_]*=' "$dir/.kamal/secrets" || fail "$name: .kamal/secrets isn't a secrets file"
	printf '#!/bin/bash\n' >"$dir/db-init.sh"
	: >"$dir/nats.conf"
	# The names the file reads from the environment, to give them values.
	grep -o '^[A-Z_][A-Z0-9_]*=\$[A-Z_][A-Z0-9_]*$' "$dir/.kamal/secrets" | cut -d= -f1 >"$dir/names"
	[ -s "$dir/names" ] || fail "$name: the secrets file reads nothing from the environment"
done

# Every secret a configuration's file refers to, with a placeholder value,
# in a subshell of its own so one configuration's values never reach the
# other's.
with_secrets() {
	local dir="$1"
	shift
	(
		cd "$dir"
		while IFS= read -r variable; do
			export "$variable=placeholder-$variable"
		done <names
		"$@"
	)
}

for name in managed_kamal managed_services; do
	with_secrets "$work/$name" kamal config --config-file release.yml --version check >"$work/$name/config.out"
	pass "$name: kamal config loads and validates release.yml"
done

dir="$work/managed_services"
with_secrets "$dir" ruby -rkamal -rjson -e '
config = Kamal::Configuration.create_from(config_file: Pathname.new("release.yml"), version: "check")
report = {}
config.roles.each do |role|
  app = Kamal::Commands::App.new(config, role: role, host: role.hosts.first)
  entry = {
    "run" => app.run.flatten.map(&:to_s).join(" "),
    "secrets" => role.secrets_io(role.hosts.first).string,
  }
  entry["proxy"] = role.proxy.deploy_command_args(target: "container").join(" ") if role.running_proxy?
  report[role.name] = entry
end
puts JSON.pretty_generate(report)
' >"$dir/roles.json"
cat "$dir/roles.json"

python3 - "$dir/roles.json" <<'EOF'
import json, sys

roles = json.load(open(sys.argv[1]))
# Kamal quotes option values; the checks read them without the quotes.
for role in roles.values():
    for key in ("run", "proxy"):
        if key in role:
            role[key] = role[key].replace('"', "")


def check(condition, message):
    if not condition:
        raise SystemExit(f"FAIL {message}")
    print(f"PASS {message}")


check(sorted(roles) == ["service-analytics", "service-cards", "web"], f"the roles are the app's and each process service's: {sorted(roles)}")
web, analytics, cards = roles["web"], roles["service-analytics"], roles["service-cards"]

# Each role's options become docker run flags on the kamal network.
for name, role, alias in (
    ("web", web, "notes-production-app"),
    ("service-analytics", analytics, "notes-production-analytics"),
    ("service-cards", cards, "notes-production-cards"),
):
    check("--network kamal" in role["run"], f"{name} runs on the kamal network")
    check(f"--network-alias {alias}" in role["run"], f"{name} gets the network alias {alias}")
    check("--read-only" in role["run"], f"{name} runs read-only")

check(analytics["run"].rstrip().endswith("/app/services/analytics"), "service-analytics runs its own command")
check(
    "--volume /var/lib/leptoskit/notes-production/services/analytics/exports:/volumes/exports" in analytics["run"],
    "service-analytics mounts its volume",
)

# `NAME:SECRET` aliases: each service reads its own secrets under the
# contract's names, and nothing of the app's.
secrets = {
    name: dict(line.split("=", 1) for line in role["secrets"].splitlines() if "=" in line)
    for name, role in roles.items()
}
check(
    secrets["service-analytics"].get("LEPTOSKIT_SERVICE_TOKEN") == "placeholder-LEPTOSKIT_SERVICE_ANALYTICS_TOKEN",
    "service-analytics reads its token as LEPTOSKIT_SERVICE_TOKEN",
)
check(
    secrets["service-analytics"].get("DATABASE_URL") == "placeholder-LEPTOSKIT_SERVICE_ANALYTICS_DATABASE_URL",
    "service-analytics reads its own database URL as DATABASE_URL",
)
check(
    secrets["service-cards"].get("LEPTOSKIT_SERVICE_TOKEN") == "placeholder-LEPTOSKIT_SERVICE_CARDS_TOKEN",
    "service-cards reads its own token",
)
for name in ("service-analytics", "service-cards"):
    leaked = sorted(set(secrets[name]) & {"DATABASE_PASSWORD", "POSTGRES_PASSWORD", "AUTH_KEY", "LEPTOSKIT_SERVICE_BILLING_TOKEN"})
    check(not leaked, f"{name} receives none of the app's secrets {leaked}")
check(secrets["web"].get("DATABASE_URL") == "placeholder-DATABASE_URL", "the app reads its own database URL")
check(
    secrets["web"].get("ANALYTICS_AUTHORIZATION") == "Bearer placeholder-LEPTOSKIT_SERVICE_ANALYTICS_TOKEN",
    "the app's token-bearing binding is filled in from the token",
)

# kamal-proxy keeps the public prefixes on the app's host.
check("proxy" in analytics, "service-analytics is behind kamal-proxy")
# Kamal splits the comma-separated list into one flag per prefix.
for prefix in ("/_leptoskit/services/analytics", "/_analytics"):
    check(f"--path-prefix={prefix} " in analytics["proxy"], f"service-analytics routes {prefix}")
check("--path-prefix" not in web["proxy"], "the app takes every other path on the host")
check("--strip-path-prefix=false" in analytics["proxy"], "service-analytics keeps the prefix on its paths")
check("--host=app.example.com " in analytics["proxy"], "service-analytics shares the app's host")
check("--target=container:3100 " in analytics["proxy"], "kamal-proxy sends service-analytics its own port")
check("--health-check-path=/readyz " in analytics["proxy"], "kamal-proxy checks service-analytics at /readyz")
check("proxy" not in cards, "service-cards, private, isn't behind kamal-proxy")
EOF
