# KubeHound gap analysis — more implicit permissions to model?

A cross-check of this catalogue (derived from Yang et al., CCS '25) against
**DataDog KubeHound**'s attack-graph edge set, to find implicit-permission
mechanisms worth adding as woodpecker transitions.

- Source: `DataDog/KubeHound` @ `ba4cdde2fa17875932c8941c3932da2e1c80eca3`
  (2026-09-14), `docs/reference/attacks/` + `pkg/kubehound/graph/edge/`.
- Our catalogue: 63 transitions / 47 mechanisms in [`transitions/`](transitions/).

KubeHound and this catalogue model different things and overlap in the middle.
KubeHound is an **attack graph** over a live cluster: its edges include runtime
exploitation (container escapes, host-mount reads) and node/pod-ownership steps
that assume code execution. This catalogue models **RBAC implicit permissions**:
an explicitly-granted API verb `p` that makes the control plane perform an
un-granted API operation `q` on the caller's behalf — exactly what a woodpecker
transition `(p ⟹ q)` encodes. The comparison is therefore scoped to KubeHound
edges that are RBAC-expressible as `permission ⟹ permission`.

## Verdict

Three findings worth modeling, one of them a new **edge class** the current
transition model does not natively express:

1. **`impersonate` → assume identity** (KubeHound `IDENTITY_IMPERSONATE`) — a
   whole RBAC-escalation family absent from this catalogue and from Yang et al.
2. **`get`/`list secrets` → assume the ServiceAccount whose token is a Secret**
   (`TOKEN_LIST`, `TOKEN_BRUTEFORCE`) — the *secret → identity* escalation; we
   model the mint path (`serviceaccounts/token`) but not the read-a-token-Secret
   path.
3. **`patch pods` → read the pod's mounted Secrets** (`POD_PATCH`) — a verb
   sibling of our `pods/exec` and `pods/ephemeralcontainers` edges.
   **DONE** (2026-10-11): added as
   [`pod-patch-image-reads-secret`](transitions/pod-patch-image-reads-secret.md),
   `[sound]` and proven live on kind.

(1) and (2) are the same shape — **identity assumption** (`→ Identity` in
KubeHound): the attacker gains *all* of a target identity's permissions, not one
specific `(action, resource)`. That is not a single `p ⟹ q` transition; see
*Modeling note* below.

## Edge-by-edge

| KubeHound edge | KubeHound shape | In this catalogue? | Disposition |
|---|---|---|---|
| `POD_EXEC` | PermissionSet→Pod | **yes** — `pod-exec-reads-secret` (`pods/exec` ⟹ get secret) | covered |
| `POD_PATCH` | PermissionSet→Pod | **yes** — `pod-patch-image-reads-secret` (`patch pods` image-swap → code-exec → mounted secret); also `pods/ephemeralcontainers`, `update pods` | **done** |
| `CONTAINER_ATTACH` | Pod→Container (`pods/debug`/ephemeral) | partial — `pod-ephemeralcontainers-updates-pod` | covered (verb sibling) |
| `ROLE_BIND` (+`crb_cr_cr`,`crb_cr_r`,`rb_rb_r`) | PermissionSet→PermissionSet | **yes** — `rolebinding-binds-role`, `clusterrolebinding-grants-all` | covered; KubeHound's scope taxonomy (RB→ClusterRole grants only namespace scope; CRB→ClusterRole grants cluster-wide) is a possible **refinement** |
| `IDENTITY_IMPERSONATE` | PermissionSet→Identity | **no** | **ADD** (new family) |
| `TOKEN_LIST` | PermissionSet→Identity (`list secrets`) | **no** (we have mint-token, not read-token-Secret) | **ADD** (identity assumption) |
| `TOKEN_BRUTEFORCE` | PermissionSet→Identity (`get secrets`) | **no** | **ADD** (identity assumption) |
| `IDENTITY_ASSUME` | Container/Node→Identity (own a token) | conceptually — `serviceaccount-token-is-sa-credential` (mint ⟹ act as SA) | covered (RBAC side); the runtime side is out of scope |
| `PERMISSION_DISCOVER` | Identity→PermissionSet (`get rolebindings`) | n/a | out of scope — reconnaissance (a read, no privilege gain) |
| `POD_CREATE` | PermissionSet→**Node** | partial — we model the *data* edges (`create pods` ⟹ get secret/configmap/sa/pvc/node, mint SA token) | node *takeover* is escape-mediated → out of scope |
| `POD_ATTACH` | **Node**→Pod | — | out of scope — runtime (node owns its pods via containerd) |
| `SHARE_PS_NAMESPACE` | Container→Container (`shareProcessNamespace`) | — | out of scope — intra-pod runtime lateral movement (pod-spec, not an RBAC verb) |
| `TOKEN_STEAL` | **Volume**→Identity | — | out of scope — runtime (read token off a mounted volume / node fs) |
| `VOLUME_ACCESS`, `VOLUME_DISCOVER` | Node/Container→Volume | — | out of scope — runtime filesystem discovery |
| `CE_*` (6: nsenter, priv_mount, module_load, sys_ptrace, umh_core_pattern, var_log_symlink) | Container→Node | — | out of scope — container escape (privileged/caps/hostPath, kernel) |
| `EXPLOIT_HOST_READ/WRITE/TRAVERSE`, `EXPLOIT_CONTAINERD_SOCK` | Container→Node/Volume | — | out of scope — host-mount / runtime-socket exploitation |
| `ENDPOINT_EXPLOIT` (internal/external) | Endpoint→Container | — | out of scope — network service exploitation |

Net: of KubeHound's ~30 edge types, the pod/workload/bind edges that are RBAC
implicit permissions are **already covered**; the rest split into **3 new
RBAC-modelable edges** and a large **runtime/escape** set that is outside this
catalogue's remit (it models RBAC `p ⟹ q`, not code execution).

## The three additions

### 1. `impersonate` ⟹ act as the impersonated identity  *(new family)*

A principal with the `impersonate` verb on `users`/`groups`/`serviceaccounts`
(or `uids`) can set `Impersonate-User`/`-Group` headers and have the API server
authorize the request as that identity — including `system:masters`. This is a
first-class RBAC escalation
([k8s user impersonation](https://kubernetes.io/docs/reference/access-authn-authz/authentication/#user-impersonation)),
not a controller/kubelet side effect, which is why Yang et al.'s controller-focused
methodology never surfaced it. KubeHound added it as `IDENTITY_IMPERSONATE`
(coverage: *None / not implemented* — a recent addition).

Why it is not a plain `p ⟹ q`: the gained capability is *every* cube the target
identity holds, parametric over all `(action, resource)`. See *Modeling note*.

### 2. `get`/`list secrets` ⟹ assume a ServiceAccount  *(identity assumption)*

Where a ServiceAccount token is stored as a `kubernetes.io/service-account-token`
Secret (legacy auto-created tokens, or any manually-created token Secret), the
ability to read that Secret yields the SA's live bearer token and hence its full
RBAC. KubeHound splits this by verb: `TOKEN_LIST` (`list secrets`) and
`TOKEN_BRUTEFORCE` (`get secrets`, names guessed from the predictable
`<sa>-token-xxxxx` convention). This catalogue currently models only the **mint**
path (`create serviceaccounts/token ⟹ get serviceaccounts`,
`serviceaccount-token-is-sa-credential`) — not the **read-the-Secret** path.

Modelable with a derived field `core::secrets.serviceAccountName: String` (empty
unless the Secret is of type `kubernetes.io/service-account-token`), defined by
the token controller / the `kubernetes.io/service-account.name` annotation, so
the edge is `get/list secrets ⟹ act as that SA` gated on
`resource.type == "kubernetes.io/service-account-token"`. Caveat: on k8s ≥1.24
tokens are no longer auto-created as Secrets, so this edge only fires where such
Secrets exist — worth noting in the mechanism file.

### 3. `patch pods` ⟹ read the pod's mounted Secrets  *(verb sibling)*

`patch pods` permits replacing `spec.containers[*].image` on a running pod with
a backdoored image, giving code execution inside the pod and therefore its
mounted SA token and `secretRefs`. This is the same leak our
`pod-exec-reads-secret` / `pod-update-reads-secret` edges model, via a different
verb (`patch` rather than `create pods/exec` or `update pods`). Directly
modelable now with the existing `core::pods.secretRefs` derived field:
`patch pods ⟹ get secret` when `resource1.secretRefs.contains(secret.name)`.

**Status: DONE** (2026-10-11) —
[`pod-patch-image-reads-secret`](transitions/pod-patch-image-reads-secret.md):
`patch pods ⟹ get secrets` when `resource1.namespace == resource.namespace &&
resource1.secretRefs.contains(resource.name)`, verified `[sound]` and proven live
in [`e2e/tests/pod-patch-image-reads-secret.sh`](e2e/tests/pod-patch-image-reads-secret.sh)
(image swap → attacker code → mounted-Secret read). The `patch pods --> get secrets`
trigger row moved here from `pod-update-reads-secret`, whose add-a-ref path is the
*unreachable* twin (blocked by Pod immutability).

### (optional) ROLE_BIND scope refinement

Our two binding transitions assert the escalation; KubeHound additionally
encodes the scope rules (a RoleBinding to a ClusterRole grants only the binding's
namespace; a ClusterRoleBinding to a ClusterRole grants cluster-wide; CRB cannot
reference a Role). If we want the synthesized policies to carry the right scope
predicate, these can sharpen the `when` on `rolebinding-binds-role` /
`clusterrolebinding-grants-all`.

## Modeling note — identity assumption is a different edge class

A woodpecker transition is `cube ⟹ cube`, where a cube is
`(principalType, action, resource)`. `impersonate` and `read-token ⟹ assume-SA`
are `… ⟹ Identity`: the attacker acquires *all* cubes of the target identity,
parametric over every `(action, resource)` that identity holds. That is not one
target `(at', rt')`; soundly expressing it needs either

- an **identity-aliasing primitive** in woodpecker (principal P "acts as"
  identity U, so U's cube set is added to P's), or
- **per-cube enumeration** (one synthesized transition per cube U holds — exact
  but blows up with the policy set), or
- an **over-approximation** target (`impersonate ⟹ any action on any resource`)
  — honest but coarse.

KubeHound sidesteps this by having explicit `→ Identity` edges in its graph
model. This is the main structural learning: the catalogue (RBAC side effects
of controllers/kubelet) and KubeHound (full attack graph incl. identity
assumption + runtime escape) overlap on pod/workload/bind edges; KubeHound adds
the **identity-assumption** dimension, of which `impersonate` and the
secret→SA-token edges are the RBAC-relevant, worth-modeling parts.

## Recommendation

- ~~Add #3 (`patch pods ⟹ get secret`)~~ — **DONE** (2026-10-11): added as
  `pod-patch-image-reads-secret` and its ConfigMap sibling
  `pod-patch-image-reads-configmap`, both `[sound]` and proven live on kind.
- **FOLLOWUP (next): #2** — `get`/`list secrets ⟹ assume SA`. Add a
  `core::secrets.serviceAccountName: String` derived field (empty unless the
  Secret is a `kubernetes.io/service-account-token`, from the
  `kubernetes.io/service-account.name` annotation), then the transition
  `get/list secrets ⟹ act as that SA` gated on
  `resource.type == "kubernetes.io/service-account-token"`. Document the ≥1.24
  caveat (tokens no longer auto-created as Secrets, so the edge only fires where
  such Secrets exist). This is an *identity-assumption* edge — see the Modeling
  note; the simplest honest encoding targets the SA's `get serviceaccounts` cube,
  as the token-mint twin `serviceaccount-token-is-sa-credential` already does.
- Treat #1 (`impersonate`) as the trigger to decide how woodpecker should model
  identity assumption (aliasing primitive vs enumeration vs over-approximation)
  before encoding it — it is the highest-impact escalation and the one that most
  stresses the current `cube ⟹ cube` model.
