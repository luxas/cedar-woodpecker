# create/update/patch DaemonSet → create/update/patch/delete Pod, ControllerRevision

**Paper:** Yang et al., CCS '25 §3.3 (the real-world case the paper centers on); severity **Critical** (Pod rows), **Low** (ControllerRevision) (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create daemonsets --> create pods`
- `update daemonsets --> create pods`
- `update daemonsets --> update pods`
- `update daemonsets --> patch pods`
- `update daemonsets --> delete pods`
- `patch daemonsets --> create pods`
- `patch daemonsets --> patch pods`
- `patch daemonsets --> delete pods`
- `delete daemonsets --> delete pods`
- `deletecollection daemonsets --> delete pods`
- `create daemonsets --> create controllerrevisions`
- `update daemonsets --> create controllerrevisions`
- `update daemonsets --> delete controllerrevisions`
- `delete daemonsets --> delete controllerrevisions`

## The implicit permission

A principal holding create/update/patch (p) on `apps::daemonsets` in a namespace
can cause create/update/patch/delete (q) on `core::pods` and
`apps::controllerrevisions` in that namespace, with no direct Pod verb. The
DaemonSet controller stamps one Pod per matching node straight from
`ds.Spec.Template`, and records each template revision as a ControllerRevision.

## Why it exists (Kubernetes mechanism)

The DaemonSet controller in `kube-controller-manager` reconciles DaemonSets by
creating a Pod on each node that should run the daemon — built directly from the
DaemonSet's pod template — and deleting Pods on nodes that should not. Revisions
are stored as ControllerRevisions via the shared history helper.
Source: [`NewPod`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/daemon/daemon_controller.go#L1451) — `newPod := &v1.Pod{Spec: ds.Spec.Template.Spec, ObjectMeta: ds.Spec.Template.ObjectMeta}`: the Pod's spec IS the DaemonSet's template, including `serviceAccountName`.
Source: [`DaemonSetsController.syncNodes`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/daemon/daemon_controller.go#L1040) — creates/deletes the daemon Pods across nodes.
Source: [`realHistory.CreateControllerRevision`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/history/controller_history.go#L307) — the ControllerRevision write.

## Transition function

Uses derived field(s): none at the DaemonSet→Pod hop (namespace-scoped). The
secret-mount escalation it chains into uses `core::pods.serviceAccountRef` /
`core::pods.secretRefs` (FAMILY A).

`daemonset-create-creates-pod.json`:
```json
{ "transitions": [ {
  "name": "daemonset-create-creates-pod",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "apps::daemonsets" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "core::pods" },
  "when": "resource.namespace == context.resource1.namespace"
} ] }
```

`daemonset-create-creates-controllerrevision.json`:
```json
{ "transitions": [ {
  "name": "daemonset-create-creates-controllerrevision",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "apps::daemonsets" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "apps::controllerrevisions" },
  "when": "resource.namespace == context.resource1.namespace"
} ] }
```

## Example

Explicit policy (`daemonset-create-creates-pod.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is apps::daemonsets
);
```

## Synthesized implicit policy

```
// daemonset-create-creates-pod: (principal: k8s::User, action: k8s::Action::"create", resource: apps::daemonsets) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.namespace == context.resource1.namespace)
@woodpecker("daemonset-create-creates-pod: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
```

```
// daemonset-create-creates-controllerrevision: (principal: k8s::User, action: k8s::Action::"create", resource: apps::daemonsets) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: apps::controllerrevisions) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.namespace == context.resource1.namespace)
@woodpecker("daemonset-create-creates-controllerrevision: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is apps::controllerrevisions
);
```

## Modeling notes

- **CVE-2024-43403 (the paper's marquee case, §3.3).** The paper formalizes
  `(patch DaemonSet, get Secret) ∈ R`: a principal granted `patch daemonsets` can
  edit the DaemonSet's pod template to mount a high-privilege ServiceAccount; the
  DaemonSet controller then creates Pods from that modified template, and each
  Pod's kubelet projects the SA token into the Pod. Via that token the attacker
  reads Secrets they were never granted — a full privilege escalation from the
  benign-looking `patch daemonsets`. In code, [`NewPod`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/daemon/daemon_controller.go#L1451)
  copies `ds.Spec.Template.Spec` (including `ServiceAccountName`) verbatim into the Pod.
- **The chain, explicitly.** This transition proves the first hop
  (`patch/create DaemonSet → create Pod`, same namespace). The second hop is
  FAMILY A's `create Pod → get Secret` keyed on `core::pods.serviceAccountRef` /
  `core::pods.secretRefs`. Composed, they are the CVE-2024-43403 escalation:
  `patch DaemonSet → (create Pod) → get Secret`. Whoever controls the workload
  controls the pod template, hence the pod's SA/secret mounts — which is why this
  Family-B mechanism is the gateway into Family A.
- **Over-approximation:** the DaemonSet→Pod hop is modeled to the namespace
  restriction (woodpecker drops the per-node / owner-ref specificity, which the
  authorizer cannot see anyway).
- **Severity:** Pod rows **Critical** (SA-token escalation above); the
  ControllerRevision side-effect **Low** (template history disclosure).
