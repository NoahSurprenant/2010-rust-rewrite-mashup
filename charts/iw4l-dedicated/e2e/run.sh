#!/usr/bin/env bash
# End-to-end test of the chart on a real cluster: ./run.sh
#
# Needs kubectl, helm and openssl, pointed at an empty cluster that enforces NetworkPolicies (k3d or
# k3s). Runs the dedicated server image IMAGE and the master image MASTER_IMAGE (repository:tag;
# default the published :latest of each). There is no MW2 in CI (it is never published), and the
# server needs it to load its match, so the release runs with ci/minimal-values.yaml's emptyDir in
# its place, beside a real master (charts/iw4l-master, with a certificate made here):
#   1. no MW2: in a namespace that enforces Pod Security's restricted profile, the pod is admitted
#      under the chart's security context, and never gets ready (its room never opens);
#   2. the pod's hardening, its probe and its NetworkPolicy: the Deployment patched so that an init
#      container (the master's image, the pod's security context) runs `iw4l-master status` against
#      the master's Service, trusting the pin - out through this chart's policy and in through the
#      master's, as the server connects - and the server's container writes the room state the
#      server writes when its room is open, then sleeps. The probe passes and the pod gets ready; in
#      it, user 65532, a read-only root filesystem and /games/mw2, /data writable, no token;
#   3. a restart: the old pod goes and a new one gets ready (Recreate: never two at once).
# Not here: the server loading MW2 and hosting its room, which the fork's dedicated_server scenario
# (crates/approved_tests) checks with the owner's MW2 files on the in-cluster runner.
set -euo pipefail

chart=$(cd "$(dirname "$0")/.." && pwd)
master_chart=$(cd "$chart/../iw4l-master" && pwd)
master_ns=iw4l-e2e
ns=iw4l-e2e-dedicated
release=iw4l-dedicated
image=${IMAGE:-ghcr.io/noahsurprenant/iw4l-dedicated:latest}
master_image=${MASTER_IMAGE:-ghcr.io/noahsurprenant/iw4l-master:latest}
deploy=deploy/$release
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

step() { echo; echo "=== $*"; }
fail() { echo "FAIL: $*" >&2; diagnose; exit 1; }
diagnose() {
  for n in "$master_ns" "$ns"; do
    echo "--- pods in $n" >&2
    kubectl -n "$n" get pods -o wide >&2 || true
    for pod in $(kubectl -n "$n" get pods -o name 2>/dev/null); do
      echo "--- $pod" >&2
      kubectl -n "$n" describe "$pod" | sed -n '/^Events:/,$p' >&2 || true
      kubectl -n "$n" logs "$pod" --all-containers --tail=60 >&2 || true
    done
  done
}
# the newest pod (an old one may still be going)
pod_name() {
  kubectl -n "$ns" get pods -l app.kubernetes.io/instance=$release --sort-by=.metadata.creationTimestamp -o name \
    | tail -n 1 | sed 's|^pod/||'
}

step "the master ($master_image)"
for n in "$master_ns" "$ns"; do
  kubectl create namespace "$n"
  kubectl label namespace "$n" pod-security.kubernetes.io/enforce=restricted pod-security.kubernetes.io/enforce-version=latest
done
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 30 \
  -keyout "$work/key.pem" -out "$work/tls.crt" -subj /CN=iw4l-prod -addext subjectAltName=DNS:iw4l-prod \
  -addext basicConstraints=critical,CA:FALSE 2>/dev/null
openssl pkcs8 -topk8 -nocrypt -in "$work/key.pem" -out "$work/tls.key"
kubectl -n "$master_ns" create secret tls iw4l-master-e2e-tls --cert "$work/tls.crt" --key "$work/tls.key"
pin=$(openssl x509 -in "$work/tls.crt" -pubkey -noout | openssl pkey -pubin -outform der | sha256sum | cut -d' ' -f1)
helm install iw4l-master "$master_chart" -n "$master_ns" -f "$master_chart/ci/minimal-values.yaml" \
  --set "image.repository=${master_image%:*}" --set "image.tag=${master_image##*:}" \
  --set probes.readiness.periodSeconds=5
kubectl -n "$master_ns" rollout status deploy/iw4l-master --timeout=180s || fail "the master did not get ready"
master_ip=$(kubectl -n "$master_ns" get svc iw4l-master -o jsonpath='{.spec.clusterIP}')
echo "ready at $master_ip:4433, pin $pin"

step "install ($image): no MW2"
helm install "$release" "$chart" -n "$ns" -f "$chart/ci/minimal-values.yaml" \
  --set "image.repository=${image%:*}" --set "image.tag=${image##*:}" --set image.pullPolicy=IfNotPresent \
  --set "master.keySha256=$pin" --set probes.readiness.initialDelaySeconds=2 --set probes.readiness.periodSeconds=5 \
  --set-json 'networkPolicy.master.to=[{"namespaceSelector": {"matchLabels": {"kubernetes.io/metadata.name": "'"$master_ns"'"}}, "podSelector": {"matchLabels": {"app.kubernetes.io/name": "iw4l-master"}}}]' \
  --set terminationGracePeriodSeconds=5
pod=
for _ in $(seq 1 90); do
  pod=$(pod_name)
  [ -z "$pod" ] || [ "$(kubectl -n "$ns" get pod "$pod" -o jsonpath='{.status.containerStatuses[0].imageID}')" = "" ] || break
  sleep 2
done
[ -n "$pod" ] || fail "no pod (refused under the restricted profile?)"
# a minute in which its room never opens
for _ in $(seq 1 30); do
  [ "$(kubectl -n "$ns" get pod "$pod" -o jsonpath='{.status.containerStatuses[0].ready}')" != true ] \
    || fail "the server got ready without MW2"
  sleep 2
done
kubectl -n "$ns" logs "$pod" --tail=20 || true
echo "admitted under the restricted profile, never ready: $pod"

step "hardening, probe and policy: the server swapped for its room state, an init container checking the master"
security=$(kubectl -n "$ns" get "$deploy" -o jsonpath='{.spec.template.spec.containers[0].securityContext}')
kubectl -n "$ns" patch "$deploy" --type=json -p "[
  {\"op\": \"add\", \"path\": \"/spec/template/spec/containers/0/command\",
   \"value\": [\"sh\", \"-c\", \"printf 'state=hosting\\\\nroom=1\\\\nmembers=1\\\\nin_match=true\\\\n' > /tmp/master.status; exec sleep infinity\"]},
  {\"op\": \"add\", \"path\": \"/spec/template/spec/initContainers\", \"value\": [{\"name\": \"master\",
    \"image\": \"$master_image\", \"securityContext\": $security,
    \"args\": [\"status\", \"--connect\", \"$master_ip:4433\", \"--server-name\", \"iw4l-prod\", \"--key-sha256\", \"$pin\"]}]}]"
kubectl -n "$ns" rollout status "$deploy" --timeout=180s || fail "the pod did not get ready (the master unreachable, or the probe)"
pod=$(pod_name)
kubectl -n "$ns" logs "$pod" -c master
in_pod() { kubectl -n "$ns" exec "$pod" -c iw4l-dedicated -- "$@"; }
[ "$(in_pod id -u)" = 65532 ] || fail "not running as 65532"
if in_pod sh -c 'touch /usr/local/bin/x' 2>/dev/null; then fail "the root filesystem is writable"; fi
if in_pod sh -c 'touch /games/mw2/x' 2>/dev/null; then fail "/games/mw2 is writable"; fi
if in_pod test -e /var/run/secrets/kubernetes.io/serviceaccount/token; then fail "a service account token is mounted"; fi
in_pod sh -c 'touch /data/x && rm /data/x' || fail "/data is not writable"
[ "$(in_pod sh -c 'echo $IW4L_MASTER_ADDR')" = "iw4l-master.$master_ns.svc.cluster.local:4433" ] || fail "IW4L_MASTER_ADDR"
kubectl -n "$ns" get networkpolicy "$release" >/dev/null || fail "no NetworkPolicy"
echo "the master reached through both policies; ready on the room state; uid 65532, read-only root and MW2, /data writable, no token"

step "restart: the old pod goes, a new one gets ready"
kubectl -n "$ns" rollout restart "$deploy"
kubectl -n "$ns" rollout status "$deploy" --timeout=180s || fail "the new pod did not get ready"
[ "$(pod_name)" != "$pod" ] || fail "no new pod"
[ "$(kubectl -n "$ns" get pods -l app.kubernetes.io/instance=$release -o name | wc -l)" = 1 ] || fail "more than one server pod"
echo "new pod $(pod_name) ready"

step "uninstall"
helm uninstall "$release" -n "$ns" --wait
helm uninstall iw4l-master -n "$master_ns" --wait
echo
echo PASS
