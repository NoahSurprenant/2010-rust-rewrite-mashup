# iw4l-dedicated Helm chart

Deploys IW4L's dedicated server ([docs/DEDICATED.md](../../docs/DEDICATED.md)):
`iw4l dedicated <zone>`, a game with no player of its own that keeps one room open on a master,
headless (no window, no GPU, no sound), so friends can play the Minecraft map with nobody hosting
from their own PC. One pod runs the server; it only connects out, to the master, which relays every
player's packets, so there is no Service. A NetworkPolicy lets nothing in.

The master is not part of this chart: [iw4l-master](../iw4l-master/README.md), in the same cluster or
anywhere else, given by its address, certificate label and key pin as players give it.

MW2's game data is **not** in the image and never will be: you mount your own copy, read-only.

Published to `oci://ghcr.io/noahsurprenant/charts/iw4l-dedicated` by the
[Helm charts workflow](../../.github/workflows/helm-charts.yml) whenever the chart changes on main,
as `<major>.<minor>.<run number>` (major and minor from `Chart.yaml`), and signed keyless with cosign
(the same identity as iw4l-master's, README there). The image,
`ghcr.io/noahsurprenant/iw4l-dedicated`, is built from
[packaging/iw4l-dedicated/Dockerfile](../../packaging/iw4l-dedicated/Dockerfile) by the
[Dedicated server image workflow](../../.github/workflows/dedicated-image.yml): `sha-<commit>` and
`latest`. Run the tag of the commit the master and the players' build came from.

## Quick start

```yaml
image:
  tag: sha-<commit>

master:
  # The iw4l-master chart's Service, in namespace iw4l-master.
  address: iw4l-master.iw4l-master.svc.cluster.local:4433
  serverName: iw4l-prod
  keySha256: <the master's pin>

room:
  name: Minecraft            # listed whole, up to 48 bytes

gameData:
  nfs:
    server: nfs.example.com
    path: /exports/games/mw2

networkPolicy:
  master:
    to:
      - namespaceSelector:
          matchLabels:
            kubernetes.io/metadata.name: iw4l-master
        podSelector:
          matchLabels:
            app.kubernetes.io/name: iw4l-master
```

```sh
helm install iw4l-dedicated oci://ghcr.io/noahsurprenant/charts/iw4l-dedicated --version <version> \
  -n iw4l-dedicated --create-namespace -f values.yaml
kubectl -n iw4l-dedicated logs -f deploy/iw4l-dedicated
```

The first start fetches Minecraft's files from Mojang (about 260 MB) and loads the map, a minute or
more; the pod is ready once the room is open, and players see it in the menu's server browser.

Every value is described in [values.yaml](values.yaml); [values.schema.json](values.schema.json)
checks them.

## Volumes

| Value | Mounted at | What |
| --- | --- | --- |
| `gameData.existingClaim`, `gameData.nfs`, `gameData.volume` (`subPath`) | `/games/mw2`, read-only | MW2's multiplayer data (`IW4L_GAMES=/games`). Exactly one source; the chart refuses to render without. |
| `persistence` (a claim of the chart's, 5Gi, `storageClass`, `existingClaim`; or an emptyDir) | `/data` | The working directory: the Minecraft worlds (`/data/minecraft-worlds/<minecraft.world>`: seed, block changes, every player's inventory), `settings.cfg`, logs, and the Minecraft files. |
| `cache.existingClaim`, `cache.volume` (`subPath`) | `/data/iw4l-artifacts` | Optional: the Minecraft files and the asset cache on a volume of their own. |
| `extraVolumes`, `extraVolumeMounts` | | Anything else. |

The world is saved every ten seconds when something changed, when a player leaves and on SIGTERM, so
a rollout or a node drain keeps it. Deleting the claim starts a new world.

## Settings

| Value | Variable |
| --- | --- |
| `zone` | `iw4l dedicated <zone>` (`minecraft:overworld`) |
| `master.address`, `master.serverName`, `master.keySha256` | `IW4L_MASTER_ADDR`, `IW4L_MASTER_SERVER_NAME`, `IW4L_MASTER_KEY_SHA256` (address and pin required) |
| `room.name` | `IW4L_MASTER_HOST_NAME`: at most 48 bytes, listed whole (the browser's column shows 22 characters, the line above the list the focused room's whole name) |
| `room.maxPlayers` | `IW4L_MASTER_MAX_PLAYERS`: 2 to 18, the server's own slot included; 0 the game's 18 |
| `room.password.existingSecret`, `.key` | `IW4L_MASTER_PASSWORD`, from the Secret |
| `minecraft.world` | `IW4L_MINECRAFT_WORLD` |
| `adminKeys` | `IW4L_ADMIN_KEYS`: the player keys (fingerprints, as `whoami` prints them) allowed debug commands (`mc give`, `mc fill`, `mc summon`, `tp`, `admin transfer`, ...); empty, none |

Always set: `IW4L_MASTER_STATUS_FILE=/tmp/master.status`, which the probe reads. The image sets
`IW4L_GAMES=/games` and the paths under `/data`. Any other setting goes in `extraEnv`, which comes
last and so overrides the chart's.

## Networking

The server opens one QUIC connection to the master and hosts its room there; players reach it
through the master's relay. NetworkPolicy, on by default:

| Direction | Allowed |
| --- | --- |
| In | Nothing (`networkPolicy.extraIngress` adds rules). |
| Out | DNS (`networkPolicy.dns.to`); UDP to the port of `master.address`, to `networkPolicy.master.to` (empty: anywhere; a Service's address is translated to its pod's before the policy sees it, so name the master's pods); TCP 443 to public addresses (`networkPolicy.internet`: Mojang's file servers), the private ranges excepted. |

A master with its own NetworkPolicy must let the server's pod in: the iw4l-master chart's does by
default.

## The pod

- `replicas: 1` (or 0 to stop it), `strategy: Recreate`: one world, one room, and a ReadWriteOnce
  volume.
- Non-root (65532), read-only root filesystem, every capability dropped, no privilege escalation,
  seccomp `RuntimeDefault`, no service account token; `fsGroup` makes `/data` writable.
- Readiness: the room is open on the master (`state=hosting` in the status file). No liveness probe by
  default: the server exits by itself (status 1) when the master drops its room (a master restart, a
  lost connection) or its match ends, and the kubelet starts it again with a new room. SIGTERM saves
  and exits 0 in well under a second (`terminationGracePeriodSeconds: 30`).
- Measured in the fork's `dedicated_server` scenario: 2.1-2.3 GiB resident, 20-30% of a core (64%
  while it loads). Requests 250m and 2560Mi, limits memory to 4Gi, no CPU limit.
- The image is amd64 only (`nodeSelector`).

## Tests

- `unittests/`: helm-unittest suites (`helm unittest -f 'unittests/*_test.yaml' charts/iw4l-dedicated`).
- `ci/*-values.yaml`: linted and rendered against the Kubernetes schemas of the oldest and newest
  supported versions.
- `e2e/run.sh`: installs the chart on a k3s cluster with the published image, beside a real master
  (the iw4l-master chart), in namespaces that enforce Pod Security's `restricted` profile. There is
  no MW2 in CI, so `ci/minimal-values.yaml` mounts an emptyDir in its place, and the test checks the
  pod is admitted, never gets ready, and that the server says it found no MW2 zone under `/games`. Then it patches the Deployment: an init container runs
  `iw4l-master status` against the master's Service, trusting its pin, through both charts'
  NetworkPolicies, and the server's container writes the room state the server writes once its room
  is open, then sleeps. The pod gets ready on its probe; the test checks its hardening and a restart.
  The server hosting a real room needs MW2, so the fork's `dedicated_server` scenario
  (`crates/approved_tests`) checks it on the in-cluster runner.

Kubernetes 1.34 to 1.37 are tested (`kubeVersion` in `Chart.yaml` refuses older).
