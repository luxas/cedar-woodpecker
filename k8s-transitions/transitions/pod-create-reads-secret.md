# create Pod → get Secret

**Paper:** Yang et al., CCS '25 §4 (implicit permissions of workload creation); severity **Critical** (classification.txt, `# Critical` bucket).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create pods --> get secrets`

This is the marquee implicit permission: creating a Pod lets you read any Secret
in the Pod's namespace, which was never granted explicitly.

## The implicit permission

Let *p* = `create pods` and *q* = `get secrets`. A principal holding *p* can
author a Pod whose `spec` references a Secret by name (image-pull secret, an
`env`/`envFrom` `secretKeyRef`, a `volumes[].secret`, or a projected volume).
The kubelet then mounts that Secret's contents into the running container, so
the Pod author gains the Secret's bytes without ever holding *q*. The implicit
edge is scoped to the Pod's own namespace and the Secrets it names.

## Why it exists (Kubernetes mechanism)

The Node authorizer builds an authorization graph in which a Pod vertex gains an
edge to each Secret it references; a node may `get` a Secret only if some Pod
bound to it points at that Secret. The set of referenced Secret names is exactly
what `VisitPodSecretNames` walks (imagePullSecrets, every container's env/envFrom,
and every volume source). The kubelet reads those Secrets to populate the
container, which is the capability the Pod author inherits.

Source: [`Graph.AddPod`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L361) — adds the pod→secret edges (the Secret loop around [L398](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L398)).
Source: [`VisitPodSecretNames`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/api/v1/pod/util.go#L113) — enumerates every Secret name a PodSpec references; this defines the `core::pods.secretRefs` derived field.

Kubernetes' own docs warn that the ability to create Pods in a namespace is
effectively the ability to read every Secret in it; this transition formalizes
that warning. The workload variant — `patch daemonsets --> get secrets`
(CVE-2024-43403) — reaches the same edge indirectly via a controller template.

## Transition function

Uses derived field(s): `core::pods.secretRefs` (see SCHEMA.md).

`pod-create-reads-secret.json`:
```json
{ "transitions": [ {
  "name": "pod-create-reads-secret",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::pods" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "core::secrets" },
  "when": "context.resource1.namespace == resource.namespace && context.resource1.secretRefs.contains(resource.name)"
} ] }
```

## Example

Explicit policy (`pod-create-reads-secret.cedar`):
```cedar
// Alice may create Pods in any namespace.
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
```

## Synthesized implicit policy

```
// pod-create-reads-secret: (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::secrets) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.resource1.namespace == resource.namespace) && context.resource1.secretRefs.contains(resource.name)
@woodpecker("pod-create-reads-secret: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::secrets
);
```

## Modeling notes

- The deep spec walk (`spec.volumes[].secret.secretName`, `env[].valueFrom`,
  `imagePullSecrets[]`, projected sources) is projected to the flat
  `core::pods.secretRefs: Set<String>` — see SCHEMA.md. The `when` keeps the
  precise over-approximation: the implicit `get Secret` is granted only for a
  Secret whose name is in `secretRefs` and only in the Pod's namespace. If a
  future Pod-admission policy constrained `secretRefs`, this would tighten
  automatically.
- Severity Critical: Secrets routinely hold TLS keys, DB passwords, and
  ServiceAccount tokens, so this edge is a direct namespace-wide credential read.
- Real-world CVE: **CVE-2024-43403** is the workload variant — `patch`/`update`
  on a controller (DaemonSet/StatefulSet) whose template names a Secret reaches
  the same kubelet mount edge indirectly.
