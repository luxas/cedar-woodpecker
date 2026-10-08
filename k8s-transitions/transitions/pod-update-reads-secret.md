# update/patch Pod → get Secret

**Paper:** Yang et al., CCS '25 §4 (implicit permissions of workload mutation); severity **Critical** (classification.txt, `# Critical` bucket).
Reference(s) formalized (from `assessment/trigger.txt`):
- `update pods --> get secrets`
- `patch pods --> get secrets`

`update` and `patch` are the same mutation mechanism (one source), so they share
this file.

## The implicit permission

Let *p* = `update pods` (or `patch pods`) and *q* = `get secrets`. A principal
holding *p* can mutate an existing Pod so that it references a Secret by name
(most of a Pod's spec is immutable after create, but the Secret-bearing surfaces
reachable by update/patch — e.g. where mutable — still cause the kubelet to
mount the named Secret). The author thereby reads the Secret without holding *q*.
The edge is scoped to the Pod's namespace and the Secrets it then names.

## Why it exists (Kubernetes mechanism)

The capability path is identical to create: the Node authorizer graph keys off
the Pod's referenced Secret set, and the kubelet mounts those Secrets. Whether
the reference arrives via `create` or a later `update`/`patch`, the resulting
Pod→Secret edge and the mount are the same.

Source: [`Graph.AddPod`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L361) — rebuilds the pod→secret edges from the current Pod (Secret loop at [L398](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L398)).
Source: [`VisitPodSecretNames`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/api/v1/pod/util.go#L113) — enumerates the referenced Secret names (the `core::pods.secretRefs` derived field).

## Transition function

Uses derived field(s): `core::pods.secretRefs` (see SCHEMA.md).

`pod-update-reads-secret.json`:
```json
{ "transitions": [ {
  "name": "pod-update-reads-secret",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"update\"", "resource": "core::pods" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "core::secrets" },
  "when": "context.resource1.namespace == resource.namespace && context.resource1.secretRefs.contains(resource.name)"
} ] }
```

## Example

Explicit policy (`pod-update-reads-secret.cedar`):
```cedar
// Alice may update (and patch) Pods in any namespace.
permit (
  principal is k8s::User,
  action in [k8s::Action::"update", k8s::Action::"patch"],
  resource is core::pods
);
```

## Synthesized implicit policy

```
// pod-update-reads-secret: (principal: k8s::User, action: k8s::Action::"update", resource: core::pods) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::secrets) [sound]
// path: (context.action1 == k8s::Action::"update") && (context.resource1.namespace == resource.namespace) && context.resource1.secretRefs.contains(resource.name)
@woodpecker("pod-update-reads-secret: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::secrets
);
```

## Modeling notes

- The transition's single source uses `k8s::Action::"update"`; `patch` is the
  same mechanism (a patch is a server-side update) and the example policy permits
  both. The synthesized `when` therefore shows `context.action1 == …"update"`
  but subsumes the `patch pods --> get secrets` row too.
- Same over-approximation as the create twin: deep spec references are projected
  to `core::pods.secretRefs`, and the implicit `get` is confined to those named
  Secrets in the Pod's namespace.
- Severity Critical: this is the mutation path to the same namespace-wide Secret
  read as `create pods --> get secrets`; it also underlies the controller-template
  CVE **CVE-2024-43403** (`patch`/`update` a workload whose template names a
  Secret).
