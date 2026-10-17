# iw4l-master Helm chart

Deploys IW4L's master server ([docs/MASTER.md](../../docs/MASTER.md)): the lobby browser and QUIC
packet relay that IW4L games host and join rooms through. A relay, not a game server - the host's
own game simulates the match - so it needs no game data and no GPU, and keeps nothing on disk. One
pod runs the master; a Service exposes its UDP port; a NetworkPolicy lets in only that port. A room
that stays open with nobody hosting from their own PC is the
[iw4l-dedicated chart](../iw4l-dedicated/README.md).

Published to `oci://ghcr.io/noahsurprenant/charts/iw4l-master` by the
[Helm charts workflow](../../.github/workflows/helm-charts.yml) whenever the chart changes on main,
as `<major>.<minor>.<run number>` (major and minor from `Chart.yaml`), and signed keyless with cosign:

```sh
cosign verify ghcr.io/noahsurprenant/charts/iw4l-master:<version> \
  --certificate-identity-regexp '^https://github\.com/NoahSurprenant/2010-rust-rewrite-mashup/\.github/workflows/helm-charts\.yml@refs/heads/main$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

The image, `ghcr.io/noahsurprenant/iw4l-master`, is built from
[packaging/iw4l-master/Dockerfile](../../packaging/iw4l-master/Dockerfile) by the
[Master image workflow](../../.github/workflows/master-image.yml): `sha-<commit>` and `latest`. Run
the tag of the commit your players' build came from: a master with another `master_protocol` version
is refused at the handshake.

## Quick start

With [cert-manager](https://cert-manager.io) in the cluster, the chart makes the master's certificate
itself:

```yaml
image:
  tag: sha-<commit>          # the players' build's commit

service:
  type: LoadBalancer
  # The players' own addresses: the master caps connections per address.
  externalTrafficPolicy: Local
```

```sh
helm install iw4l-master oci://ghcr.io/noahsurprenant/charts/iw4l-master --version <version> \
  -n iw4l-master --create-namespace -f values.yaml
kubectl -n iw4l-master logs deploy/iw4l-master | grep key-sha256
```

Forward UDP 4433 on the router to the Service, and give players `IW4L_MASTER_ADDR=<public
address>:4433`, `IW4L_MASTER_SERVER_NAME=iw4l-prod` and `IW4L_MASTER_KEY_SHA256=<the pin>`
(docs/MASTER.md, "Hand out to players"). NOTES.txt prints the commands again.

Every value is described in [values.yaml](values.yaml); [values.schema.json](values.schema.json)
checks them, so a misspelled or unknown key, or a value of the wrong type, fails the install instead
of being ignored.

## The certificate and the pin

Players trust the master by the SHA-256 of its public key, so **the key must outlive the release**.
Two ways to give the master one, mounted at `/etc/iw4l/tls`:

| Value | Certificate |
| --- | --- |
| `tls.certManager.enabled` (default) | cert-manager makes it: a self-signed Issuer (`<release>-selfsigned`) and a Certificate (`<release>`) for `server.serverName`, ECDSA P-256 in PKCS#8, ten years, into the Secret `tls.certManager.secretName` (`<release>-tls`). `privateKey.rotationPolicy: Never`: a renewal re-signs the same key. `issuerRef` uses an issuer of your own instead. |
| `tls.existingSecret` | A Secret you made (`tls.crt`, `tls.key` in PKCS#8), e.g. with openssl as docs/MASTER.md shows. The chart makes no certificate. |

cert-manager owns its Secret, and, unless it runs with `--enable-certificate-owner-ref`, does not
delete it with the Certificate: uninstalling the release keeps the key, and a release installed
again with the same names takes it over, pin and all. Deleting the Secret makes a new key and a new
pin for every player.

The master reads the files only at start: after a renewal, restart it (or let a reloader do it).

## Networking

One UDP port, `server.port` (4433, the prod channel's; 4434 is the dev channel's), exposed by the
Service on `service.port`. Most players come from the internet, so a real install is a
`LoadBalancer` (a fixed address is usually an annotation of the implementation, in
`service.annotations`) or `NodePort`, with the router forwarding the port to it.
`service.externalTrafficPolicy: Local` keeps the players' own addresses, which the master's
per-address connection cap (32) counts; with `Cluster` every player counts as the node.

### Network policy

On by default (`networkPolicy.enabled`):

| Direction | Allowed |
| --- | --- |
| In | UDP on `server.port`, from anyone: players and pods in the cluster alike (a dedicated server). `networkPolicy.from` narrows it. |
| Out | Nothing: the master only answers connections made to it, and looks no names up. |

`networkPolicy.extraIngress` and `extraEgress` add rules.

## The pod

- One replica with `strategy: Recreate`: rooms live in the process's memory, so two pods would be two
  lobbies, and they would share one address.
- Non-root (65532, the distroless image's), read-only root filesystem, every capability dropped, no
  privilege escalation, seccomp `RuntimeDefault`, no service account token. The key is mounted 0440,
  group-owned by `fsGroup`.
- Probes: `iw4l-master status` against itself over loopback, trusting only the mounted certificate
  (or the issuer's CA with `tls.certManager.issuerRef`) for `server.serverName`: an expired or wrong
  certificate fails it. Each check is a connection in the log, so the periods are long (readiness
  60 s, liveness 120 s).
- Up to 256 connections; a few MiB idle. Requests 20m and 32Mi, limits memory to 256Mi, and sets no
  CPU limit: the relay must not be throttled mid-match.
- The image is a static amd64 binary: amd64 nodes only (`nodeSelector`).

## Tests

- `unittests/`: [helm-unittest](https://github.com/helm-unittest/helm-unittest) suites
  (`helm unittest -f 'unittests/*_test.yaml' charts/iw4l-master`; not `tests/`, which
  `make publish-check` keeps to `crates/approved_tests`).
- `ci/*-values.yaml`: linted and rendered against the Kubernetes schemas (cert-manager's from the CRDs
  catalog) of the oldest and newest supported versions.
- `e2e/run.sh`: installs the chart on a k3s cluster with the published image, in a namespace that
  enforces Pod Security's `restricted` profile. With a Secret made by openssl, the master gets ready
  on its own probes and logs the certificate's pin, and a pod in another namespace runs
  `iw4l-master status` against the Service through the NetworkPolicy. Then, with cert-manager
  installed, the chart's Issuer and Certificate make an ECDSA P-256 key the master serves with, and
  after an uninstall and a new install the Secret, the key and the pin are the same.

Kubernetes 1.34 to 1.37 are tested (`kubeVersion` in `Chart.yaml` refuses older).
