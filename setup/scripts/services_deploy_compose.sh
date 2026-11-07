#!/usr/bin/env bash
# Runs the Compose project `services_deploy_fixture.sh` wrote, with the image
# already built, and checks it end to end through its Caddy edge.
#
#   services_deploy_compose.sh <project> <sqlite|postgres>
#
# The checks every deployment shares are in services_deploy_checks.py; the
# ones here are Compose's own: which ports are published, clean health
# checks and stops, the edge against a direct call, data that survives a
# restart, and on Postgres, the service's role and database.
set -euo pipefail

project="$(realpath "$1")"
dialect="$2"
scripts="$(cd "$(dirname "$0")" && pwd)"
cd "$project"

export COMPOSE_PROJECT_NAME="services-deploy-$dialect"
LEPTOSKIT_SERVICE_ANALYTICS_TOKEN="$(openssl rand -hex 32)"
LEPTOSKIT_SERVICE_ECHO_TOKEN="$(openssl rand -hex 32)"
export LEPTOSKIT_SERVICE_ANALYTICS_TOKEN LEPTOSKIT_SERVICE_ECHO_TOKEN
if [ "$dialect" = postgres ]; then
	POSTGRES_PASSWORD="$(openssl rand -hex 24)"
	LEPTOSKIT_SERVICE_ANALYTICS_DATABASE_PASSWORD="$(openssl rand -hex 24)"
	export POSTGRES_PASSWORD LEPTOSKIT_SERVICE_ANALYTICS_DATABASE_PASSWORD
fi

fail() {
	echo "FAIL $*" >&2
	exit 1
}

pass() {
	echo "PASS $*"
}

finish() {
	status=$?
	if [ "$status" -ne 0 ]; then
		docker compose ps --all || true
		docker compose logs --no-color --timestamps || true
	fi
	docker compose down --volumes --timeout 40 || true
	exit "$status"
}
trap finish EXIT

docker compose version
docker compose config --quiet
pass "the Compose file is valid"

# The image is the one the generated Dockerfile built; nothing builds here.
docker compose up --detach --wait --wait-timeout 240 --no-build
pass "docker compose up --wait: every health check passed"

# The Dockerfile's health check answers for the app, and each service's own
# check asks its /readyz.
for service in app analytics echo; do
	container="$(docker compose ps --quiet "$service")"
	health="$(docker inspect --format '{{.State.Health.Status}}' "$container")"
	[ "$health" = healthy ] || fail "$service is $health"
	pass "$service's health check is healthy"
done

python3 "$scripts/services_deploy_checks.py" \
	--base http://127.0.0.1:3000 \
	--origin http://localhost:3000 \
	--echo-trust isolated

# Only the edge is published, on loopback; the app and the services aren't.
docker compose ps --format json | python3 -c '
import json, sys
text = sys.stdin.read().strip()
rows = json.loads(text) if text.startswith("[") else [json.loads(line) for line in text.splitlines()]
published = {
    row["Service"]: sorted({(p["URL"], p["PublishedPort"]) for p in row.get("Publishers") or [] if p.get("PublishedPort")})
    for row in rows
}
assert published.pop("edge") == [("127.0.0.1", 3000)], published
assert not any(published.values()), published
'
pass "only the edge publishes a port, on loopback"
for port in 3100 3300; do
	if curl --silent --max-time 3 "http://127.0.0.1:$port/readyz" >/dev/null; then
		fail "a service answers on the host at port $port"
	fi
done
pass "the services' ports aren't reachable from the host"

# The edge stripped the cookies the echo service would otherwise see and
# set: inside the network, without the edge, they pass.
direct="$(docker compose exec -T app curl --silent --include \
	--header 'Cookie: session=the-apps-session' http://echo:3300/_leptoskit/services/echo/headers)"
grep -q '^cookie: session=the-apps-session' <<<"$direct" || fail "the echo service didn't see a direct Cookie: $direct"
grep -qi '^set-cookie: echo=' <<<"$direct" || fail "the echo service set no cookie directly: $direct"
pass "without the edge, the echo service sees and sets cookies"

# The data outlives the service's container.
summary() {
	curl --silent --fail --header 'Content-Type: application/json' --data '{"days":7}' \
		http://127.0.0.1:3000/_leptoskit/services/analytics/api/summary |
		python3 -c 'import json, sys; print(json.load(sys.stdin)["total"])'
}
before="$(summary)"
docker compose up --detach --wait --wait-timeout 120 --no-build --no-deps --force-recreate analytics
after="$(summary)"
[ "$before" = "$after" ] && [ "$after" -ge 1 ] || fail "the count was $before before recreating the service and $after after"
pass "the service's data survives recreating its container ($after events)"

if [ "$dialect" = sqlite ]; then
	docker compose exec -T analytics test -s /data/analytics.sqlite3 || fail "no database on the service's volume"
	docker volume inspect "${COMPOSE_PROJECT_NAME}_analytics-data" >/dev/null
	pass "the service's SQLite database is on its own volume"
else
	role=analytics_app_service_analytics
	psql_as() {
		docker compose exec -T --env PGPASSWORD="$2" db \
			psql --host 127.0.0.1 --username "$1" --dbname "$3" --no-psqlrc --tuples-only --no-align --command "$4"
	}
	[ "$(psql_as analytics_app "$POSTGRES_PASSWORD" analytics_app \
		"SELECT rolsuper FROM pg_roles WHERE rolname = '$role'")" = f ] || fail "the service's role is missing or a superuser"
	[ "$(psql_as analytics_app "$POSTGRES_PASSWORD" analytics_app \
		"SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname = '$role'")" = "$role" ] ||
		fail "the service's database isn't owned by its role"
	pass "the init script made the service's role and database"
	count="$(psql_as "$role" "$LEPTOSKIT_SERVICE_ANALYTICS_DATABASE_PASSWORD" "$role" 'SELECT count(*) FROM events')"
	[ "$count" = "$after" ] || fail "the service's own database holds $count events, not $after"
	pass "the service's events are in its own database"
	if refused="$(psql_as "$role" "$LEPTOSKIT_SERVICE_ANALYTICS_DATABASE_PASSWORD" analytics_app 'SELECT 1' 2>&1)"; then
		fail "the service's role connected to the app's database"
	fi
	grep -q 'permission denied for database' <<<"$refused" || fail "unexpected refusal: $refused"
	pass "the service's role can't connect to the app's database"
	[ "$(psql_as analytics_app "$POSTGRES_PASSWORD" analytics_app \
		"SELECT string_agg(rolname, ',') FROM pg_roles WHERE rolname LIKE 'analytics\_app\_service\_%'")" = "$role" ] ||
		fail "a service without a database got a role"
	pass "a service without a database has no role"
fi

# `docker compose stop` sends SIGTERM; each process drains and exits 0
# within its grace period, rather than being killed.
docker compose stop --timeout 40
for service in app analytics echo; do
	container="$(docker compose ps --all --quiet "$service")"
	code="$(docker inspect --format '{{.State.ExitCode}}' "$container")"
	[ "$code" = 0 ] || fail "$service exited with $code after SIGTERM"
done
pass "the app and its services stop cleanly on SIGTERM"
