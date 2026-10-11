# create/update/patch StatefulSet → create/delete Pod, ControllerRevision, PersistentVolumeClaim

**Paper:** Yang et al., CCS '25 §3.3 (controller reconciliation); severity **Critical** (Pod rows), **Low** (ControllerRevision / PVC side-effects) (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create statefulsets --> create pods`
- `update statefulsets --> create pods`
- `update statefulsets --> update pods`
- `update statefulsets --> patch pods`
- `update statefulsets --> delete pods`
- `patch statefulsets --> create pods`
- `patch statefulsets --> patch pods`
- `patch statefulsets --> delete pods`
- `delete statefulsets --> delete pods`
- `deletecollection statefulsets --> delete pods`
- `create statefulsets --> create controllerrevisions`
- `update statefulsets --> create controllerrevisions`
- `update statefulsets --> delete controllerrevisions`
- `delete statefulsets --> delete controllerrevisions`
- `create statefulsets --> create persistentvolumeclaims`
- `delete statefulsets --> delete persistentvolumeclaims`
- `deletecollection statefulsets --> delete persistentvolumeclaims`

## The implicit permission

A principal holding create/update/patch (p) on `apps::statefulsets` in a namespace
can cause create/update/patch/delete (q) on `core::pods`,
`apps::controllerrevisions`, and `core::persistentvolumeclaims` in that namespace,
with no direct verb on any of those three. The StatefulSet controller stamps
ordinal Pods from the pod template, records each revision as a ControllerRevision
(for rolling updates / history), and provisions one PVC per
`volumeClaimTemplates` entry per Pod.

## Why it exists (Kubernetes mechanism)

The StatefulSet controller in `kube-controller-manager` creates each Pod and,
before the Pod, the PersistentVolumeClaims the Pod needs (from
`spec.volumeClaimTemplates`). It stores each pod-template revision as a
ControllerRevision via the shared history helper.
Source: [`StatefulPodControl.CreateStatefulPod`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/statefulset/stateful_pod_control.go#L154) — creates the Pod after its PVCs.
Source: [`StatefulPodControl.createPersistentVolumeClaims`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/statefulset/stateful_pod_control.go#L381) — creates the PVCs from the volume-claim templates.
Source: [`realHistory.CreateControllerRevision`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/history/controller_history.go#L307) — calls `ControllerRevisions(ns).Create(...)`.

## Transition function

Uses derived field(s): none (namespace-scoped reconciliation).

`statefulset-create-creates-pod.json`:
```json
{ "transitions": [ {
  "name": "statefulset-create-creates-pod",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "apps::statefulsets" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "core::pods" },
  "when": "resource.namespace == context.resource1.namespace"
} ] }
```

`statefulset-create-creates-controllerrevision.json`:
```json
{ "transitions": [ {
  "name": "statefulset-create-creates-controllerrevision",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "apps::statefulsets" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "apps::controllerrevisions" },
  "when": "resource.namespace == context.resource1.namespace"
} ] }
```

`statefulset-create-creates-pvc.json`:
```json
{ "transitions": [ {
  "name": "statefulset-create-creates-pvc",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "apps::statefulsets" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "core::persistentvolumeclaims" },
  "when": "resource.namespace == context.resource1.namespace"
} ] }
```

## Example

Explicit policy (`statefulset-create-creates-pod.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is apps::statefulsets
);
```

## Synthesized implicit policy

```
// statefulset-create-creates-pod: (principal: k8s::User, action: k8s::Action::"create", resource: apps::statefulsets) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.namespace == context.resource1.namespace)
@woodpecker("statefulset-create-creates-pod: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
```

```
// statefulset-create-creates-controllerrevision: (principal: k8s::User, action: k8s::Action::"create", resource: apps::statefulsets) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: apps::controllerrevisions) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.namespace == context.resource1.namespace)
@woodpecker("statefulset-create-creates-controllerrevision: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is apps::controllerrevisions
);
```

```
// statefulset-create-creates-pvc: (principal: k8s::User, action: k8s::Action::"create", resource: apps::statefulsets) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::persistentvolumeclaims) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.namespace == context.resource1.namespace)
@woodpecker("statefulset-create-creates-pvc: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::persistentvolumeclaims
);
```

## Modeling notes

- **Over-approximation:** namespace restriction as elsewhere — all three implicit
  permissions are granted for any such resource in the StatefulSet's namespace.
- **Severity split:** the Pod reconciliation is **Critical** for the same
  template-controls-pod reason (chains into FAMILY A `create Pod → get Secret`
  via the pod template's `serviceAccountName`/secret refs). The ControllerRevision
  and PVC side-effects are **Low** on their own — a ControllerRevision stores the
  serialized pod template (information disclosure of what is already the
  principal's own template), and the auto-provisioned PVC is a storage side-effect
  — but the PVC path can bind a `core::persistentvolumeclaims` whose
  `storageClassName` triggers dynamic PV provisioning (see FAMILY StorageClass→PV).
