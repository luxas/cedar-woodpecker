# Plan 17 — Kubernetes implicit permissions as woodpecker transition functions

## Goal

Take the implicit-permission catalogue of Yang et al. (CCS '25, the PDF added in
this branch's first commit) and formalize it as `cedar-woodpecker` transition
functions over the real Kubernetes Cedar schema, one markdown per mechanism,
each confirming *why* the implicit permission exists with a permalink into
Kubernetes source, and each runnable one validated by `cedar-woodpecker
escalate` (synthesized implicit policy must be `sound`).

The paper's relation `(p, q) ∈ R` is exactly a transition
`(pt, [(at_i, rt_i)], at', rt', T)`: source = explicit `p`, target = implicit
`q`, `T` = the system rule. The deliverable lives in `k8s-transitions/`.

## Schema

`kubernetes-cedar-authorizer` generates a Cedar schema from the Kubernetes
OpenAPI. We generate it offline (reproducibly) from the JSON corpora in the
Kubernetes source tree with `tools/schema_gen_offline.rs`, against
`kubernetes/kubernetes@788c4eca67a3ba878ead2385db1f4e274f9fd3a9` and
`kubernetes-cedar-authorizer@c1ce0f60`. One upstream dedup bug (OpenAPI importer
only checked `common_types`, not `entity_types`, so `meta::V1ObjectMeta`
collided when combining groups) is fixed in the driver's build.

A critical property of an implicit permission is often buried in a nested,
optional object body (a Pod's Secret refs). Per the task, we add **derived
logical fields** — a flat projection on the resource entity, defined by the
exact Kubernetes function that computes it — so the mechanism is explicit and a
future Pod-admission policy has a place to constrain it (proving the path does
not in practice grant the access). These would be an extension function
(`secretRefs(PodSpec) -> Set<String>`) in future. All derived fields are in
`SCHEMA.md`; `k8s-base.cedarschema` is unedited, `k8s.cedarschema` has them.

## Families (one subagent each, one or more markdowns per family)

1. **Pod resource references** — `create Pod → get {Secret, ConfigMap,
   ServiceAccount, PersistentVolumeClaim, Node}`; `update/patch Pod → get
   {Secret, ConfigMap}`; pod subresources `pods/exec`, `pods/attach →
   get Secret`, `pods/eviction → delete Pod`, `pods/ephemeralcontainers →
   update Pod`. The marquee `create Pod → get Secret` (CVE-2024-43403 family).
2. **Workload controllers → Pods** — Deployment, ReplicaSet, StatefulSet,
   DaemonSet, ReplicationController, Job, CronJob, their `/scale` subresources,
   ControllerRevisions, StatefulSet PVCs, HorizontalPodAutoscaler.
3. **RBAC & identity** — `ClusterRoleBinding → *`, `RoleBinding → bind Role`,
   `serviceaccounts/token → get ServiceAccount`, `csr/approval → approve`,
   `delete ServiceAccount → delete Secret`, `create Pod → serviceaccounts/token`.
4. **Namespace lifecycle** — `delete Namespace → delete <all namespaced>`
   (cascade), `create Namespace → create {ServiceAccount, ConfigMap}`,
   `namespaces/finalize`.
5. **Networking & storage** — `Service → Endpoints/EndpointSlices`,
   `APIService → get/watch Services`, `StorageClass → create PersistentVolume`,
   `FlowSchema → get PriorityLevelConfiguration`, servicecidr, volumeattachment.
6. **Subresource → parent** — the generic `X/status`, `X/scale → X` family
   (get/update/patch parent), and `patch Secret → patch Pod` reverse mount.
7. **Deny-side & admission** (documented, `NOT SYNTHESIZABLE`) — ResourceQuota,
   ValidatingAdmissionPolicy, MutatingWebhookConfiguration, CRD → CR.

## Per-markdown contract

Every `transitions/<name>.md` has: title + the paper reference and severity;
the explicit → implicit statement; the trigger.txt rows it subsumes; **Why**
(mechanism prose + a `kubernetes@788c4ec` permalink to the controller/kubelet/
apiserver code); **Transition** (the JSON, and which derived fields it uses);
**Example** (an explicit policy); **Synthesized** (woodpecker's implied policy,
pasted from a real run, `sound`); **Modeling notes** (over-approximations,
schema caveats). Runnable ones carry `<name>.json` (+ `.cedar`); the whole set
is in `transitions.json` and checked by `verify.sh`.

## Verification

`./verify.sh` runs `cedar-woodpecker escalate` for every runnable transition and
asserts the synthesized policy is `sound`; it is the gate for this branch.

## Deliberately not done

Transitive multi-hop chains beyond what a single transition's source list
expresses; deny-side synthesis (woodpecker synthesizes permits only); the `* *`
RBAC target is modeled against the catch-all `k8s::Resource` with a documented
caveat rather than quantified over every type.
