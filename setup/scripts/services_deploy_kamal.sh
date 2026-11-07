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

for name in managed_kamal managed_services; do
	dir="$work/$name"
	mkdir -p "$dir/.kamal"
	body "$snapshots/leptoskit_cli__infra__tests__$name.snap" >"$dir/release.yml"
	body "$snapshots/leptoskit_cli__infra__tests__${name/managed_kamal/managed_secret_references}.snap" >"$dir/.kamal/secrets"
	[ -s "$dir/release.yml" ] && [ -s "$dir/.kamal/secrets" ] || fail "$name: empty snapshot"
	printf '#!/bin/bash\n' >"$dir/db-init.sh"
	: >"$dir/nats.conf"

	# Every secret the file refers to, with a placeholder value.
	while IFS= read -r variable; do
		export "$variable=placeholder-$variable"
	done < <(grep -o '^[A-Z_][A-Z0-9_]*=\$[A-Z_][A-Z0-9_]*$' "$dir/.kamal/secrets" | cut -d= -f1)

	(cd "$dir" && kamal config -c release.yml) >"$dir/config.out"
	pass "$name: kamal config loads and validates release.yml"
done

dir="$work/managed_services"
(cd "$dir" && ruby -rkamal -rjson -e '
config = Kamal::Configuration.create_from(config_file: Pathname.new("release.yml"), version: "check")
report = {}
config.roles.each do |role|
  app = Kamal::Commands::App.new(config, role: role, host: role.hosts.first)
  entry = {
    "run" => app.run.flatten.map(&:to_s).join(" "),
    "secrets" => role.secrets_io.string,
  }
  entry["proxy"] = Kamal::Utils.optionize(role.proxy.deploy_options).join(" ") if role.running_proxy?
  report[role.name] = entry
end
puts JSON.pretty_generate(report)
') >"$dir/roles.json"
cat "$dir/roles.json"

python3 - "$dir/roles.json" <<'EOF'
import json, sys

roles = json.load(open(sys.argv[1]))


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
check(
    "--path-prefix=\"/_leptoskit/services/analytics,/_analytics\"" in analytics["proxy"]
    or "--path-prefix=/_leptoskit/services/analytics,/_analytics" in analytics["proxy"],
    "service-analytics routes its public prefixes",
)
check("--strip-path-prefix=false" in analytics["proxy"], "service-analytics keeps the prefix on its paths")
check("app.example.com" in analytics["proxy"], "service-analytics shares the app's host")
check("proxy" not in cards, "service-cards, private, isn't behind kamal-proxy")
EOF
