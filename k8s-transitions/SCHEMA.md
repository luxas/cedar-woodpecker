# The Cedar schema these transitions reason over

`cedar-woodpecker` reasons about a *typed* world: every resource is a Cedar
entity, every verb a Cedar action, and a transition's condition `T` is a Cedar
expression typechecked against the schema. This folder uses the schema that the
[`kubernetes-cedar-authorizer`](https://github.com/luxas/kubernetes-cedar-authorizer)
project generates from the live Kubernetes OpenAPI, plus a handful of *derived
logical fields* that this catalogue adds (explained below).

## Two files

- `k8s-base.cedarschema` — the schema exactly as `kubernetes-cedar-authorizer`
  emits it, with **no edits**. This is the "initial cedarschema" referred to in
  the task.
- `k8s.cedarschema` — `k8s-base.cedarschema` **plus the derived fields** in the
  table below. Every transition in `transitions/` is validated against this file.

## How the base schema was generated

`kubernetes-cedar-authorizer`'s schema generator normally talks to a live
cluster. For reproducibility we ran it offline against the two JSON corpora
checked into the Kubernetes source tree (`api/discovery/*.json`,
`api/openapi-spec/v3/*_openapi.json`) with a small offline driver,
`tools/schema_gen_offline.rs` (a drop-in `src/bin/` main for that repo). One
upstream dedup bug had to be fixed to combine more than one API group in one
run: the OpenAPI importer skipped a type already present as a *common type* but
not one already present as an *entity type* (e.g. `meta::V1ObjectMeta`, which
the base fragment defines as an entity), which collided at merge. The driver was
built against that one-line fix (also noted in the follow-ups).

Provenance of this generation:

- Kubernetes source: `kubernetes/kubernetes` @ `788c4eca67a3ba878ead2385db1f4e274f9fd3a9`
  (the local `kubernetes` checkout). All WHY permalinks in `transitions/` pin
  this commit.
- `kubernetes-cedar-authorizer` @ `c1ce0f60ea3656989adb64a3f57475fe7d131fff`.
- API groups included: core `v1`, `apps/v1`, `batch/v1`, `autoscaling/v2`,
  `rbac.authorization.k8s.io/v1`, `certificates.k8s.io/v1`,
  `apiextensions.k8s.io/v1`, `admissionregistration.k8s.io/v1`,
  `flowcontrol.apiserver.k8s.io/v1`, `storage.k8s.io/v1`,
  `apiregistration.k8s.io/v1`, `storagemigration.k8s.io/v1`,
  `scheduling.k8s.io/v1`, `node.k8s.io/v1`, `networking.k8s.io/v1`, `policy/v1`,
  `coordination.k8s.io/v1`, `discovery.k8s.io/v1`.

## Names you need when writing a transition

- **Actions** live in the `k8s` namespace: `k8s::Action::"create"`,
  `k8s::Action::"get"`, `"update"`, `"patch"`, `"delete"`, `"deletecollection"`,
  `"list"`, `"watch"`, and the RBAC-only verbs `"bind"`, `"escalate"`,
  `"approve"`, `"use"`, `"impersonate"`. Every action is `in [k8s::Action::"*"]`.
- **Resources** are entities named by API group then plural kind. The core group
  is the `core` namespace (`core::pods`, `core::secrets`, `core::configmaps`,
  `core::serviceaccounts`, `core::persistentvolumeclaims`, `core::namespaces`,
  `core::replicationcontrollers`, `core::services`, `core::endpoints`,
  `core::resourcequotas`, `core::pods_exec`, `core::pods_eviction`,
  `core::serviceaccounts_token`, …). Other groups use their reverse-DNS
  namespace: `apps::deployments`, `apps::daemonsets`, `apps::statefulsets`,
  `apps::replicasets`, `apps::controllerrevisions`, `batch::jobs`,
  `batch::cronjobs`, `autoscaling::horizontalpodautoscalers`,
  `io::k8s::authorization::rbac::clusterrolebindings` (and `rolebindings`,
  `roles`, `clusterroles`), `io::k8s::certificates::certificatesigningrequests`,
  `io::k8s::admissionregistration::validatingadmissionpolicies` (and the webhook
  configs), `io::k8s::apiserver::flowcontrol::flowschemas` (and
  `prioritylevelconfigurations`), `io::k8s::storage::storageclasses` (and
  `volumeattachments`), `io::k8s::apiregistration::apiservices`,
  `io::k8s::apiextensions::customresourcedefinitions`,
  `policy::poddisruptionbudgets`, `io::k8s::discovery::endpointslices`,
  `io::k8s::networking::ingresses`.
- A subresource is one entity, named `<kind>_<subresource>`:
  `core::pods_status`, `core::pods_exec`, `core::replicationcontrollers_scale`,
  `apps::deployments_scale`, `core::namespaces_finalize`.
- **Principals** applicable to every action: `k8s::User`, `k8s::ServiceAccount`,
  `k8s::Node`, `k8s::UnauthenticatedUser`. Only `User`, `ServiceAccount` and
  `Node` carry `groups`/`extra`; `UnauthenticatedUser` is attribute-free, so a
  policy or `when` that reads `principal.groups` must scope the principal
  (`principal is k8s::User`) or it will not typecheck in the
  `UnauthenticatedUser` environment.
- A resource entity carries `name: String`, `apiGroup: String`,
  `resourceCombined: String`, and (if namespaced) `namespace: k8s::Namespace`
  (an entity, with `.name`). The request/stored object bodies hang off
  `request?: Versioned…` / `stored?: Versioned…`.

## Derived logical fields

A critical property of an implicit permission is often buried deep in an
optional, nested object body — a Pod's Secret references live under
`spec.volumes[].secret.secretName`, `spec.containers[].env[].valueFrom`,
`spec.imagePullSecrets[]`, and more. Reasoning over that directly is both
undecidable in general and unavailable to an authorizer, which only sees request
*metadata*. Following the task's guidance, we instead add a **flat logical
projection** to the resource entity — the set/scalar that the implicit
permission actually turns on — and define it by the exact Kubernetes function
that computes it. This makes the mechanism explicit *and* gives a future
Pod-admission policy a place to constrain it (so one could prove `create Pod`
does **not** in practice grant `get Secret` for a given Secret). In the future
these would be a Cedar extension function, e.g. `secretRefs(PodSpec) -> Set<String>`,
rather than a stored attribute.

All line numbers pin `kubernetes/kubernetes@788c4eca67a3ba878ead2385db1f4e274f9fd3a9`.

| Entity | Field | Type | Means | Defined by (Kubernetes) |
|---|---|---|---|---|
| `core::pods` | `secretRefs` | `Set<String>` | names of Secrets the pod references (imagePullSecrets, env/envFrom, volumes, projected) | [`VisitPodSecretNames`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/api/v1/pod/util.go#L113) |
| `core::pods` | `configMapRefs` | `Set<String>` | names of ConfigMaps the pod references | [`VisitPodConfigmapNames`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/api/v1/pod/util.go#L204) |
| `core::pods` | `pvcRefs` | `Set<String>` | names of PersistentVolumeClaims the pod references | [`Graph.AddPod` PVC loop](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L421) |
| `core::pods` | `resourceClaimRefs` | `Set<String>` | names of ResourceClaims the pod references | [`resourceclaim.PodClaims`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/dynamic-resource-allocation/resourceclaim/pod.go#L35) |
| `core::pods` | `serviceAccountRef` | `String` | `Spec.ServiceAccountName` | [`Graph.AddPod` SA edge](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L385) |
| `core::pods` | `nodeName` | `String` | `Spec.NodeName` (scheduling target) | [`Graph.AddPod` node edge](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L361) |
| `core::serviceaccounts` | `secretRefs` | `Set<String>` | Secrets holding this SA's auto-generated legacy tokens | [legacy token cleaner](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/serviceaccount/legacy_serviceaccount_token_cleaner.go) |
| `core::persistentvolumes` | `storageClassName` | `String` | `Spec.StorageClassName` | PV spec |
| `core::persistentvolumes` | `claimRef` | `String` | `Spec.ClaimRef.Name` (bound PVC) | PV spec |
| `core::persistentvolumeclaims` | `storageClassName` | `String` | `Spec.StorageClassName` | PVC spec |
| `core::persistentvolumeclaims` | `volumeName` | `String` | `Spec.VolumeName` (bound PV) | PVC spec |
| `autoscaling::horizontalpodautoscalers` | `scaleTargetKind` | `String` | `Spec.ScaleTargetRef.Kind` | HPA spec |
| `autoscaling::horizontalpodautoscalers` | `scaleTargetName` | `String` | `Spec.ScaleTargetRef.Name` | HPA spec |
| `io::k8s::apiserver::flowcontrol::flowschemas` | `priorityLevelName` | `String` | `Spec.PriorityLevelConfiguration.Name` | FlowSchema spec |

Each field is marked in `k8s.cedarschema` with a `// … [derived, see SCHEMA.md]`
comment so a reader can tell the generated schema apart from these additions.
