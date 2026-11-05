#!/usr/bin/env bash
# Add the GitHub trusted publisher for every publishable crate.
#
# The script is additive: it never edits or removes a trusted publisher that
# already exists. That is deliberate. When a repository moves between owners,
# the old publisher has to stay in place until a release from the new location
# has proved that the new one works.
#
# crates.io only accepts a trusted publisher for a crate that has been
# published at least once. A crate that is not published yet is reported and
# left alone; publish it once with a token, then run this script again.
#
# The crates.io API allows about one request a second. The default delay keeps
# the script inside that limit; override it with --delay or
# TRUSTED_PUBLISHER_DELAY when the registry is quiet.
#
# Usage:
#   CRATES_IO_TOKEN=... setup/scripts/enroll_trusted_publishers.sh
#   CRATES_IO_TOKEN=... setup/scripts/enroll_trusted_publishers.sh --dry-run
#   CRATES_IO_TOKEN=... setup/scripts/enroll_trusted_publishers.sh --crate leptoskit
#
# CRATES_IO_TOKEN needs the `trusted-publishing` scope (a legacy unrestricted
# token also works). The token is never printed.

set -euo pipefail

owner="leptoskit"
repo="leptoskit"
workflow="publish.yml"
environment="publisher"
delay="${TRUSTED_PUBLISHER_DELAY:-1}"
dry_run=false
selected=()

usage() {
	cat <<'EOF'
Add the GitHub trusted publisher for every publishable crate.

Crates are discovered from `cargo metadata`; pass --crate to limit the run to
specific crates. Existing publishers are never changed or removed.

Options:
  --owner <owner>              GitHub repository owner (default: leptoskit)
  --repo <repo>                GitHub repository name (default: leptoskit)
  --workflow <file>            Workflow filename (default: publish.yml)
  --environment <name>         GitHub environment (default: publisher)
  --crate <name>               Only this crate; repeat for more than one
  --delay <seconds>            Delay between API requests (default: 1)
  --dry-run                    Report what would be added, change nothing
  -h, --help                   Show this help text

The CRATES_IO_TOKEN environment variable must hold a crates.io API token with
the `trusted-publishing` scope.
EOF
}

while [[ $# -gt 0 ]]; do
	case "$1" in
	--owner)
		owner="${2:?--owner needs a value}"
		shift 2
		;;
	--repo)
		repo="${2:?--repo needs a value}"
		shift 2
		;;
	--workflow)
		workflow="${2:?--workflow needs a value}"
		shift 2
		;;
	--environment)
		environment="${2:?--environment needs a value}"
		shift 2
		;;
	--crate)
		selected+=("${2:?--crate needs a value}")
		shift 2
		;;
	--delay)
		delay="${2:?--delay needs a value}"
		shift 2
		;;
	--dry-run)
		dry_run=true
		shift
		;;
	-h | --help)
		usage
		exit 0
		;;
	*)
		echo "unknown option: $1" >&2
		usage >&2
		exit 2
		;;
	esac
done

if [[ -z "${CRATES_IO_TOKEN:-}" ]]; then
	echo "CRATES_IO_TOKEN is not set." >&2
	echo "Create a token with the trusted-publishing scope at https://crates.io/settings/tokens" >&2
	exit 1
fi

# crates.io looks the owner up on GitHub when it stores a config, so the
# organization has to exist before this script can succeed.
if command -v gh >/dev/null 2>&1; then
	if ! gh api "orgs/${owner}" >/dev/null 2>&1; then
		echo "warning: GitHub organization '${owner}' is not visible to 'gh'." >&2
		echo "         Create it before adding trusted publishers, or every request will fail." >&2
	fi
fi

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

crates=()
if [[ ${#selected[@]} -gt 0 ]]; then
	crates=("${selected[@]}")
else
	while IFS= read -r crate; do
		[[ -n "$crate" ]] && crates+=("$crate")
	done < <(
		cargo metadata --no-deps --format-version 1 |
			jq -r '.packages[] | select(.publish != []) | .name' |
			sort
	)
fi

if [[ ${#crates[@]} -eq 0 ]]; then
	echo "no publishable crates found" >&2
	exit 1
fi

api="https://crates.io/api/v1"
user_agent="leptoskit-trusted-publishers (https://github.com/${owner}/${repo})"

# Keep the token out of the process table. curl reads the header from this
# private file, which the trap removes however the script exits.
curl_config="$(mktemp "${TMPDIR:-/tmp}/leptoskit-trusted-publishers.XXXXXX")"
trap 'rm -f "$curl_config"' EXIT
chmod 600 "$curl_config"
printf 'header = "Authorization: %s"\nuser-agent = "%s"\n' "$CRATES_IO_TOKEN" "$user_agent" >"$curl_config"

request() {
	curl --silent --show-error --fail-with-body --config "$curl_config" "$@"
}

pause() {
	if [[ "$delay" != "0" ]]; then
		sleep "$delay"
	fi
}

created=0
existing=0
failed=0
failures=""

echo "Trusted publisher: ${owner}/${repo}, workflow ${workflow}, environment ${environment}"
echo "Crates: ${#crates[@]}"
if [[ "$dry_run" == true ]]; then
	echo "Mode: dry run"
fi

for crate in "${crates[@]}"; do
	echo "${crate}"

	if ! configs=$(request "${api}/trusted_publishing/github_configs?crate=${crate}"); then
		echo "  could not read the existing publishers" >&2
		failed=$((failed + 1))
		failures="${failures}${crate}: could not read the existing publishers"$'\n'
		pause
		continue
	fi
	pause

	if printf '%s' "$configs" | jq -e \
		--arg owner "$owner" \
		--arg repo "$repo" \
		--arg workflow "$workflow" \
		--arg environment "$environment" \
		'.github_configs[]? | select(
			.repository_owner == $owner
			and .repository_name == $repo
			and .workflow_filename == $workflow
			and ((.environment // "") == $environment)
		)' >/dev/null; then
		echo "  already enrolled"
		existing=$((existing + 1))
		continue
	fi

	if [[ "$dry_run" == true ]]; then
		echo "  would add"
		created=$((created + 1))
		continue
	fi

	payload=$(
		jq -n \
			--arg crate "$crate" \
			--arg owner "$owner" \
			--arg repo "$repo" \
			--arg workflow "$workflow" \
			--arg environment "$environment" \
			'{github_config: {
				crate: $crate,
				repository_owner: $owner,
				repository_name: $repo,
				workflow_filename: $workflow,
				environment: $environment
			}}'
	)

	if response=$(request \
		--request POST \
		--header "Content-Type: application/json" \
		--data "$payload" \
		"${api}/trusted_publishing/github_configs"); then
		echo "  added"
		created=$((created + 1))
	else
		echo "  failed: ${response}" >&2
		failed=$((failed + 1))
		failures="${failures}${crate}: ${response}"$'\n'
	fi
	pause
done

echo
echo "created: ${created}, already enrolled: ${existing}, failed: ${failed}"

if [[ "$failed" -gt 0 ]]; then
	echo >&2
	echo "Failures:" >&2
	printf '%s' "$failures" >&2
	echo >&2
	echo "A crate that has never been published cannot be enrolled yet." >&2
	exit 1
fi
