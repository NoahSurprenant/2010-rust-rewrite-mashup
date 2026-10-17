#!/usr/bin/env bash
# End-to-end test of the chart on a real cluster: ./run.sh
#
# Needs kubectl, helm and openssl, pointed at an empty cluster that enforces NetworkPolicies (k3d or
# k3s), and runs the master image IMAGE (repository:tag; default the published :latest). The master
# needs no game data, so it runs for real:
#   1. an existing Secret (ci/minimal-values.yaml, a certificate made here with openssl, as
#      docs/MASTER.md makes it): in a namespace that enforces Pod Security's restricted profile, the
#      pod is admitted under the chart's security context, the probes (the binary's own client over
#      QUIC) pass and it gets ready; the pin it logs is the certificate's; the Service has the ready
#      pod as its endpoint;
#   2. the NetworkPolicy admits pods: a client pod in another namespace runs `iw4l-master status`
#      against the Service, trusting the pin, as a dedicated server in the cluster connects;
#   3. cert-manager (installed here): the chart's self-signed Issuer and Certificate, a key the master
#      loads and probes that pass with it;
#   4. the pin survives the release: uninstalled, cert-manager's Secret stays; installed again, the
#      new Certificate takes the Secret over without a new key, and the master logs the same pin.
set -euo pipefail

chart=$(cd "$(dirname "$0")/.." && pwd)
ns=iw4l-e2e
client_ns=iw4l-e2e-client
release=iw4l-master
image=${IMAGE:-ghcr.io/noahsurprenant/iw4l-master:latest}
cert_manager_version=${CERT_MANAGER_VERSION:-v1.21.2}
deploy=deploy/$release
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

step() { echo; echo "=== $*"; }
fail() { echo "FAIL: $*" >&2; diagnose; exit 1; }
diagnose() {
  for n in "$ns" "$client_ns"; do
    echo "--- pods in $n" >&2
    kubectl -n "$n" get pods -o wide >&2 || true
    for pod in $(kubectl -n "$n" get pods -o name 2>/dev/null); do
      echo "--- $pod" >&2
      kubectl -n "$n" describe "$pod" | sed -n '/^Events:/,$p' >&2 || true
      kubectl -n "$n" logs "$pod" --all-containers --tail=60 >&2 || true
    done
  done
  kubectl -n "$ns" get certificate,issuer,secret >&2 2>/dev/null || true
}
# the SHA-256 of a PEM certificate's public key, as players pin it
pin_of() { openssl x509 -pubkey -noout | openssl pkey -pubin -outform der | sha256sum | cut -d' ' -f1; }
# the pin the running master logged at its start (the newest pod: an old one may still be going)
logged_pin() {
  kubectl -n "$ns" logs "$(newest)" | sed -n 's/.*iw4l-master key-sha256=\([0-9a-f]*\).*/\1/p' | tail -n 1
}
secret_pin() {
  kubectl -n "$ns" get secret "$1" -o jsonpath='{.data.tls\.crt}' | base64 -d | pin_of
}
# the newest pod of the release
newest() { kubectl -n "$ns" get pods -l app.kubernetes.io/instance=$release --sort-by=.metadata.creationTimestamp -o name | tail -n 1; }
# uninstalls the release and waits for its pod to be gone (helm waits for the Deployment only)
uninstall() {
  helm uninstall "$release" -n "$ns" --wait
  kubectl -n "$ns" wait --for=delete pod -l app.kubernetes.io/instance=$release --timeout=120s
}
install() {
  helm upgrade --install "$release" "$chart" -n "$ns" \
    --set "image.repository=${image%:*}" --set "image.tag=${image##*:}" --set image.pullPolicy=IfNotPresent \
    --set probes.readiness.periodSeconds=5 "$@"
  kubectl -n "$ns" rollout status "$deploy" --timeout=180s || fail "the master did not get ready"
}

step "an existing Secret ($image)"
kubectl create namespace "$ns"
# (a pod is refused there unless it runs as non-root, without capabilities or privilege escalation,
# under the runtime's seccomp profile)
kubectl label namespace "$ns" pod-security.kubernetes.io/enforce=restricted pod-security.kubernetes.io/enforce-version=latest
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 30 \
  -keyout "$work/key.pem" -out "$work/tls.crt" -subj /CN=iw4l-prod -addext subjectAltName=DNS:iw4l-prod \
  -addext basicConstraints=critical,CA:FALSE 2>/dev/null
openssl pkcs8 -topk8 -nocrypt -in "$work/key.pem" -out "$work/tls.key"
kubectl -n "$ns" create secret tls iw4l-master-e2e-tls --cert "$work/tls.crt" --key "$work/tls.key"
want=$(pin_of < "$work/tls.crt")
install -f "$chart/ci/minimal-values.yaml"
got=$(logged_pin)
[ "$got" = "$want" ] || fail "the master logged pin '$got', not the certificate's $want"
[ -z "$(kubectl -n "$ns" get certificate -o name 2>/dev/null)" ] || fail "a Certificate was made beside an existing Secret"
[ "$(kubectl -n "$ns" get svc "$release" -o jsonpath='{.spec.ports[0].protocol}/{.spec.ports[0].port}')" = UDP/4433 ] \
  || fail "the Service does not expose UDP 4433"
ready=$(kubectl -n "$ns" get endpointslices -l kubernetes.io/service-name=$release \
  -o jsonpath='{.items[*].endpoints[?(@.conditions.ready==true)].addresses[0]}')
[ -n "$ready" ] || fail "the Service has no ready endpoint"
spec=$(kubectl -n "$ns" get "$deploy" -o jsonpath='{.spec.template.spec.securityContext.runAsUser}/{.spec.template.spec.containers[0].securityContext.readOnlyRootFilesystem}/{.spec.template.spec.automountServiceAccountToken}')
[ "$spec" = 65532/true/false ] || fail "the pod is not hardened as expected: $spec"
echo "admitted under the restricted profile, ready on its own QUIC probes; pin $got"

step "a pod in another namespace reaches the master through the NetworkPolicy"
kubectl create namespace "$client_ns"
address=$(kubectl -n "$ns" get svc "$release" -o jsonpath='{.spec.clusterIP}')
kubectl -n "$client_ns" run status --image "$image" --image-pull-policy IfNotPresent --restart Never \
  --overrides '{"spec": {"securityContext": {"runAsNonRoot": true, "runAsUser": 65532, "seccompProfile": {"type": "RuntimeDefault"}},
    "containers": [{"name": "status", "image": "'"$image"'", "args": ["status", "--connect", "'"$address"':4433", "--server-name", "iw4l-prod", "--key-sha256", "'"$want"'"],
      "securityContext": {"allowPrivilegeEscalation": false, "readOnlyRootFilesystem": true, "capabilities": {"drop": ["ALL"]}}}]}}'
for _ in $(seq 1 60); do
  phase=$(kubectl -n "$client_ns" get pod status -o jsonpath='{.status.phase}')
  case "$phase" in Succeeded|Failed) break ;; esac
  sleep 2
done
kubectl -n "$client_ns" logs status
[ "$phase" = Succeeded ] || fail "iw4l-master status from a pod: $phase"
echo "status from $client_ns, trusting the pin: ok"

step "cert-manager: the chart's self-signed Issuer and Certificate"
helm install cert-manager cert-manager --repo https://charts.jetstack.io --version "$cert_manager_version" \
  -n cert-manager --create-namespace --set crds.enabled=true --wait --timeout 5m
uninstall
install
kubectl -n "$ns" wait certificate/$release --for=condition=Ready --timeout=120s || fail "the Certificate is not ready"
[ "$(kubectl -n "$ns" get certificate $release -o jsonpath='{.spec.secretName}')" = iw4l-master-tls ] || fail "the Secret is not <release>-tls"
kubectl -n "$ns" get secret iw4l-master-tls -o jsonpath='{.data.tls\.crt}' | base64 -d > "$work/cm.crt"
openssl x509 -in "$work/cm.crt" -noout -text | grep -q 'ASN1 OID: prime256v1' || fail "the key is not ECDSA P-256"
if openssl x509 -in "$work/cm.crt" -noout -text | grep -q 'CA:TRUE'; then fail "the certificate is a CA"; fi
first=$(secret_pin iw4l-master-tls)
[ "$(logged_pin)" = "$first" ] || fail "the master logged another pin than its Secret's"
echo "ECDSA P-256, self-signed for iw4l-prod, ready; pin $first"

step "the pin survives the release"
uninstall
kubectl -n "$ns" get secret iw4l-master-tls >/dev/null || fail "uninstalling deleted cert-manager's Secret"
install
kubectl -n "$ns" wait certificate/$release --for=condition=Ready --timeout=120s || fail "the Certificate is not ready"
again=$(secret_pin iw4l-master-tls)
[ "$again" = "$first" ] || fail "the key changed: $first, now $again"
[ "$(logged_pin)" = "$first" ] || fail "the master logged another pin"
echo "the same key and pin after a reinstall: $again"

step "restart: the old pod goes, a new one gets ready"
pod=$(newest)
kubectl -n "$ns" rollout restart "$deploy"
kubectl -n "$ns" rollout status "$deploy" --timeout=180s || fail "the new pod did not get ready"
[ "$(newest)" != "$pod" ] || fail "no new pod"

step "uninstall"
uninstall
echo
echo PASS
