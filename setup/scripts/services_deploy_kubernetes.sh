#!/usr/bin/env bash
# Applies the kubernetes.yaml `services_deploy_fixture.sh` wrote to a kind
# cluster, and checks it end to end through an ingress-nginx Ingress.
#
#   services_deploy_kubernetes.sh <project> <image>
#
# The cluster is the current kubectl context, created from
# services_deploy_kind.yaml so the node's port 80 is 127.0.0.1:8080 on the
# host, and <image> is already loaded into its nodes. The manifests are
# validated with kubeconform against the cluster's version first.
set -euo pipefail

project="$(realpath "$1")"
image="$2"
scripts="$(cd "$(dirname "$0")" && pwd)"
cd "$project"
work="$(mktemp -d)"
origin=http://localhost:8080
# The last ingress-nginx release, whose manifest for kind publishes the
# controller on the node's port 80.
ingress_version=controller-v1.15.1
ingress_sha256=2a3ae008c8786431115502644e77ab398fdebfb721a5d1195ed3089cde3299df

fail() {
	echo "FAIL $*" >&2
	exit 1
}

pass() {
	echo "PASS $*"
}

# Runs a command until it succeeds, at most <attempts> times, <delay>
# seconds apart; fails when every attempt did.
retry() {
	local attempts="$1" delay="$2"
	shift 2
	for _ in $(seq 1 "$attempts"); do
		"$@" && return 0
		sleep "$delay"
	done
	return 1
}

finish() {
	status=$?
	if [ "$status" -ne 0 ]; then
		kubectl get all,ingress,pvc,secret --output wide || true
		kubectl describe pods || true
		for deployment in analytics-app analytics-app-analytics analytics-app-echo; do
			kubectl logs "deployment/$deployment" --all-containers --tail 200 || true
		done
		kubectl --namespace ingress-nginx logs deployment/ingress-nginx-controller --tail 200 || true
	fi
	rm -rf "$work"
	exit "$status"
}
trap finish EXIT

# What a deploy changes in the file: `image` points at the pushed image,
# here the one loaded into kind's nodes. With a tag other than `latest`,
# the kubelet uses it rather than pulling.
grep -qx '          image: analytics_app' kubernetes.yaml || fail "kubernetes.yaml names no image analytics_app"
sed "s|^          image: analytics_app\$|          image: $image|" kubernetes.yaml >"$work/kubernetes.yaml"

version="$(kubectl version --output json | python3 -c 'import json, sys; print(json.load(sys.stdin)["serverVersion"]["gitVersion"].lstrip("v"))')"
kubeconform -strict -summary -kubernetes-version "$version" "$work/kubernetes.yaml"
pass "the manifests validate against Kubernetes $version"

# Every container has both probes, on its own port.
python3 - "$work/kubernetes.yaml" <<'EOF'
import re, sys
text = open(sys.argv[1]).read()
for manifest in text.split("\n---\n"):
    if "\nkind: Deployment\n" not in manifest:
        continue
    port = re.search(r"containerPort: (\d+)", manifest).group(1)
    for probe, path in (("readinessProbe", "/readyz"), ("livenessProbe", "/healthz")):
        found = re.search(probe + r":\n\s+httpGet:\n\s+path: (\S+)\n\s+port: (\d+)", manifest)
        assert found and found.groups() == (path, port), (probe, manifest)
EOF
pass "every Deployment probes /readyz and /healthz on its own port"

curl --fail --silent --show-error --location --retry 3 --output "$work/ingress-nginx.yaml" \
	"https://raw.githubusercontent.com/kubernetes/ingress-nginx/$ingress_version/deploy/static/provider/kind/deploy.yaml"
echo "$ingress_sha256  $work/ingress-nginx.yaml" | sha256sum --check --strict
kubectl apply --filename "$work/ingress-nginx.yaml"
kubectl --namespace ingress-nginx rollout status deployment/ingress-nginx-controller --timeout 300s
# What a cluster's administrator sets once: the class an Ingress without
# ingressClassName gets.
kubectl annotate ingressclass nginx ingressclass.kubernetes.io/is-default-class=true
# The admission webhook answers only once its endpoint is ready.
admission_ready() {
	kubectl --namespace ingress-nginx get endpoints ingress-nginx-controller-admission \
		--output jsonpath='{.subsets[0].addresses[0].ip}' 2>/dev/null | grep -q .
}
retry 30 2 admission_ready || fail "ingress-nginx's admission webhook never got an endpoint"

# The Secrets, as the generator's next steps say to make them.
analytics_token="$(openssl rand -hex 32)"
echo_token="$(openssl rand -hex 32)"
kubectl create secret generic analytics-app \
	--from-literal=LEPTOSKIT_SERVICE_ANALYTICS_TOKEN="$analytics_token" \
	--from-literal=LEPTOSKIT_SERVICE_ECHO_TOKEN="$echo_token"
kubectl create secret generic analytics-app-analytics \
	--from-literal=LEPTOSKIT_SERVICE_TOKEN="$analytics_token" \
	--from-literal=LEPTOSKIT_PUBLIC_ORIGIN="$origin"
kubectl create secret generic analytics-app-echo \
	--from-literal=LEPTOSKIT_SERVICE_TOKEN="$echo_token" \
	--from-literal=LEPTOSKIT_PUBLIC_ORIGIN="$origin"

retry 10 5 kubectl apply --filename "$work/kubernetes.yaml" || fail "kubectl apply kept failing"
for deployment in analytics-app-analytics analytics-app-echo analytics-app; do
	kubectl rollout status "deployment/$deployment" --timeout 300s
done
pass "every Deployment rolled out, with its readiness probe passing"

# The Secrets reached the processes under the contract's names.
[ "$(kubectl exec deployment/analytics-app-analytics -- printenv LEPTOSKIT_SERVICE_TOKEN)" = "$analytics_token" ] ||
	fail "the analytics service has another token"
[ "$(kubectl exec deployment/analytics-app -- printenv LEPTOSKIT_SERVICE_ANALYTICS_TOKEN)" = "$analytics_token" ] ||
	fail "the app has another token for analytics"
[ "$(kubectl exec deployment/analytics-app-echo -- printenv LEPTOSKIT_PUBLIC_ORIGIN)" = "$origin" ] ||
	fail "the echo service has another public origin"
pass "each process reads its token and origin from its Secret"

# Only the Ingress reaches the cluster: every Service is internal.
types="$(kubectl get services --output jsonpath='{range .items[*]}{.metadata.name}={.spec.type}{"\n"}{end}' | grep -v '^kubernetes=')"
if grep -v '=ClusterIP$' <<<"$types"; then
	fail "a Service is published outside the cluster"
fi
pass "every Service is ClusterIP: only the Ingress is published"

retry 30 2 curl --silent --fail --max-time 5 --output /dev/null http://127.0.0.1:8080/readyz ||
	fail "the Ingress never answered /readyz"
python3 "$scripts/services_deploy_checks.py" \
	--base http://127.0.0.1:8080 \
	--origin "$origin" \
	--echo-trust first-party

# The service's SQLite database is on its PersistentVolumeClaim: it outlives
# the pod.
summary() {
	curl --silent --fail --header 'Content-Type: application/json' --data '{"days":7}' \
		http://127.0.0.1:8080/_leptoskit/services/analytics/api/summary |
		python3 -c 'import json, sys; print(json.load(sys.stdin)["total"])'
}
before="$(summary)"
kubectl rollout restart deployment/analytics-app-analytics
kubectl rollout status deployment/analytics-app-analytics --timeout 300s
retry 30 2 summary >/dev/null || fail "the service never answered after replacing its pod"
after="$(summary)"
[ "$before" = "$after" ] && [ "$after" -ge 1 ] || fail "the count was $before before replacing the pod and $after after"
pass "the service's data survives replacing its pod ($after events)"
