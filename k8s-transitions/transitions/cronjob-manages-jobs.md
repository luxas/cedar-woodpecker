# create/update/patch CronJob → create/update/patch/delete Job (transitively → Pod)

**Paper:** Yang et al., CCS '25 §3.3 (controller reconciliation); severity **Critical** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create cronjobs --> create jobs`
- `update cronjobs --> create jobs`
- `update cronjobs --> update jobs`
- `update cronjobs --> delete jobs`
- `patch cronjobs --> create jobs`
- `patch cronjobs --> patch jobs`
- `patch cronjobs --> delete jobs`
- `delete cronjobs --> delete jobs`
- `deletecollection cronjobs --> delete jobs`

## The implicit permission

A principal holding create/update/patch (p) on `batch::cronjobs` in a namespace
can cause create/update/patch/delete (q) on `batch::jobs` in that namespace, with
no direct Job verb. The CronJob controller creates a Job from the CronJob's
`jobTemplate` on each schedule tick (and prunes old Jobs). Because the created
Job then creates Pods (`job-manages-pods.md`), CronJob write transitively reaches
`create pods` too.

## Why it exists (Kubernetes mechanism)

The CronJob controller (v2) in `kube-controller-manager` evaluates the schedule
and, when due, materializes a Job from `cronJob.Spec.JobTemplate`.
Source: [`ControllerV2.syncCronJob`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/cronjob/cronjob_controllerv2.go#L449) — the sync loop that, at [cronjob_controllerv2.go#L632](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/cronjob/cronjob_controllerv2.go#L632), calls `jm.jobControl.CreateJob(cronJob.Namespace, jobReq)`.

## Transition function

Uses derived field(s): none (namespace-scoped reconciliation).

`cronjob-create-creates-job.json`:
```json
{ "transitions": [ {
  "name": "cronjob-create-creates-job",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "batch::cronjobs" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "batch::jobs" },
  "when": "resource.namespace == context.resource1.namespace"
} ] }
```

## Example

Explicit policy (`cronjob-create-creates-job.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is batch::cronjobs
);
```

## Synthesized implicit policy

```
// cronjob-create-creates-job: (principal: k8s::User, action: k8s::Action::"create", resource: batch::cronjobs) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: batch::jobs) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.namespace == context.resource1.namespace)
@woodpecker("cronjob-create-creates-job: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is batch::jobs
);
```

## Modeling notes

- **Over-approximation:** namespace restriction.
- **Transitivity:** this is one hop of a two-hop chain
  `CronJob → Job → Pod`. The `Job → Pod` hop lives in `job-manages-pods.md`.
  Composed, CronJob write transitively grants `create pods`, which then **chains
  into FAMILY A's `create Pod → get Secret`** via the jobTemplate's pod template
  (`serviceAccountName`/secret refs).
- **Severity Critical:** inherited from the Pod/SA-token escalation at the end of
  the chain.
