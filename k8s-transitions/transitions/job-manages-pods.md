# create Job → create/delete Pod

**Paper:** Yang et al., CCS '25 §3.3 (controller reconciliation); severity **Critical** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create jobs --> create pods`
- `delete jobs --> delete pods`
- `deletecollection jobs --> delete pods`

## The implicit permission

A principal holding create (p) on `batch::jobs` in a namespace can cause create
(and, by deleting the Job, delete) (q) on `core::pods` in that namespace, with no
direct Pod verb. The Job controller stamps Pods from the Job's pod template up to
the required completions/parallelism; deleting the Job garbage-collects its Pods.

## Why it exists (Kubernetes mechanism)

The Job controller in `kube-controller-manager` drives each Job toward its
desired number of running/succeeded Pods by creating Pods from `job.Spec.Template`.
Source: [`Controller.manageJob`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/job/job_controller.go#L1761) — the active-pod reconciliation that, at [job_controller.go#L1931](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/job/job_controller.go#L1931), calls `jm.podControl.CreatePodsWithGenerateName(ctx, job.Namespace, template, job, ...)`.

## Transition function

Uses derived field(s): none (namespace-scoped reconciliation).

`job-create-creates-pod.json`:
```json
{ "transitions": [ {
  "name": "job-create-creates-pod",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "batch::jobs" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "core::pods" },
  "when": "resource.namespace == context.resource1.namespace"
} ] }
```

## Example

Explicit policy (`job-create-creates-pod.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is batch::jobs
);
```

## Synthesized implicit policy

```
// job-create-creates-pod: (principal: k8s::User, action: k8s::Action::"create", resource: batch::jobs) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.namespace == context.resource1.namespace)
@woodpecker("job-create-creates-pod: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
```

## Modeling notes

- **Over-approximation:** namespace restriction.
- **Severity Critical:** the Job's pod template fixes `serviceAccountName`/secret
  refs, so `create Job` **chains into FAMILY A's `create Pod → get Secret`** just
  like the other workload controllers. A CronJob reaches this transitively
  (`cronjob-manages-jobs.md` → this file).
