# Index — Kubernetes implicit permissions → transition files

All **328** triggered references in
`ImplicitPermission/Combined_LLM&Static_permission_analysis/assessment/trigger.txt`
map to exactly one mechanism file below — verified programmatically, 0 unmapped
(a row's file is chosen by its *source* resource; a subresource row such as
`patch resourcequotas/status` maps to the status family, not to the primary
resource's file). A **runnable** file ships `.json` transition(s) + `.cedar`
example(s), each validated `[sound]` by [`../verify.sh`](../verify.sh)
(63 sound / 0 failed). A **not synthesizable** file documents the mechanism and
its Kubernetes WHY but ships no transition, with the reason given (woodpecker
synthesizes *permit* policies only; or the schema lacks the target entity).

One markdown documents one *mechanism*; a mechanism that spans a verb family or
several target resources (e.g. a controller reconciling its pod template) lists
every trigger row it subsumes inside the file and may ship several `.json`/`.cedar`
pairs.

## A. Pod resource references

| Mechanism | Rows | Severity | Status |
|---|---|---|---|
| [`pod-create-reads-secret.md`](pod-create-reads-secret.md) | 1 | Critical | runnable (1) |
| [`pod-create-reads-configmap.md`](pod-create-reads-configmap.md) | 1 | Medium | runnable (1) |
| [`pod-create-reads-serviceaccount.md`](pod-create-reads-serviceaccount.md) | 1 | Medium | runnable (1) |
| [`pod-create-reads-pvc.md`](pod-create-reads-pvc.md) | 1 | Medium | runnable (1) |
| [`pod-create-reads-node.md`](pod-create-reads-node.md) | 1 | Medium | runnable (1) |
| [`pod-create-reads-namespace.md`](pod-create-reads-namespace.md) | 1 | Medium | runnable (1) |
| [`pod-create-podschedulingcontexts.md`](pod-create-podschedulingcontexts.md) | 1 | Low | not synthesizable — resource.k8s.io group not in the schema |
| [`pod-update-reads-secret.md`](pod-update-reads-secret.md) | 1 | Critical | runnable (1) |
| [`pod-patch-image-reads-secret.md`](pod-patch-image-reads-secret.md) | 1 | Critical | runnable (1) |
| [`pod-patch-image-reads-configmap.md`](pod-patch-image-reads-configmap.md) | 1 | Medium | runnable (1) |
| [`pod-update-reads-configmap.md`](pod-update-reads-configmap.md) | 1 | Medium | runnable (1) |
| [`pod-exec-reads-secret.md`](pod-exec-reads-secret.md) | 1 | Critical | runnable (1) |
| [`pod-eviction-deletes-pod.md`](pod-eviction-deletes-pod.md) | 1 | Critical | runnable (1) |
| [`pod-ephemeralcontainers-updates-pod.md`](pod-ephemeralcontainers-updates-pod.md) | 3 | Critical/Medium | runnable (1) |
| [`pod-create-mints-sa-token.md`](pod-create-mints-sa-token.md) | 0 | — | runnable (1) |

## B. Workload controllers → Pods

| Mechanism | Rows | Severity | Status |
|---|---|---|---|
| [`deployment-manages-pods.md`](deployment-manages-pods.md) | 14 | Critical/High | runnable (2) |
| [`replicaset-manages-pods.md`](replicaset-manages-pods.md) | 10 | Critical/High | runnable (1) |
| [`statefulset-manages-pods.md`](statefulset-manages-pods.md) | 17 | Critical/High/Low | runnable (3) |
| [`daemonset-manages-pods.md`](daemonset-manages-pods.md) | 14 | Critical/High/Low | runnable (2) |
| [`replicationcontroller-manages-pods.md`](replicationcontroller-manages-pods.md) | 9 | Critical/High | runnable (1) |
| [`job-manages-pods.md`](job-manages-pods.md) | 3 | Critical/High | runnable (1) |
| [`cronjob-manages-jobs.md`](cronjob-manages-jobs.md) | 9 | Critical/High | runnable (1) |
| [`scale-subresource-scales-pods.md`](scale-subresource-scales-pods.md) | 16 | High | runnable (4) |
| [`hpa-scales-workloads.md`](hpa-scales-workloads.md) | 12 | High | runnable (4) |

## C. RBAC & identity

| Mechanism | Rows | Severity | Status |
|---|---|---|---|
| [`clusterrolebinding-grants-all.md`](clusterrolebinding-grants-all.md) | 3 | Critical | runnable (1) |
| [`rolebinding-binds-role.md`](rolebinding-binds-role.md) | 3 | Critical | runnable (1) |
| [`serviceaccount-token-is-sa-credential.md`](serviceaccount-token-is-sa-credential.md) | 1 | Critical | runnable (1) |
| [`csr-approval-issues-cert.md`](csr-approval-issues-cert.md) | 3 | Critical/Medium | runnable (1) |
| [`serviceaccount-delete-deletes-secret.md`](serviceaccount-delete-deletes-secret.md) | 2 | Low | runnable (1) |

## D. Namespace lifecycle

| Mechanism | Rows | Severity | Status |
|---|---|---|---|
| [`namespace-delete-cascades.md`](namespace-delete-cascades.md) | 25 | High | runnable (4) |
| [`namespace-create-seeds-defaults.md`](namespace-create-seeds-defaults.md) | 2 | Low | runnable (2) |
| [`namespace-finalize.md`](namespace-finalize.md) | 2 | Critical/High | runnable (2) |

## E. Node lifecycle

| Mechanism | Rows | Severity | Status |
|---|---|---|---|
| [`node-deletes-pods.md`](node-deletes-pods.md) | 3 | Critical/High | runnable (1) |

## F. Networking & storage

| Mechanism | Rows | Severity | Status |
|---|---|---|---|
| [`service-manages-endpoints.md`](service-manages-endpoints.md) | 10 | Low | runnable (2) |
| [`apiservice-reaches-services.md`](apiservice-reaches-services.md) | 6 | Medium | runnable (1) |
| [`storageclass-provisions-pv.md`](storageclass-provisions-pv.md) | 1 | Low | runnable (0) |
| [`flowschema-references-prioritylevel.md`](flowschema-references-prioritylevel.md) | 3 | Medium | runnable (1) |
| [`subresource-status-networking-storage.md`](subresource-status-networking-storage.md) | 0 | — | runnable (2) |

## G. Subresource → parent & reverse mount

| Mechanism | Rows | Severity | Status |
|---|---|---|---|
| [`status-subresource-updates-parent.md`](status-subresource-updates-parent.md) | 58 | Critical/Low | runnable (4) |
| [`get-status-reads-parent.md`](get-status-reads-parent.md) | 15 | Medium/Low | runnable (4) |
| [`scale-reads-parent.md`](scale-reads-parent.md) | 0 | — | not synthesizable — no triggered rows (scoping note) |
| [`secret-remounts-pod.md`](secret-remounts-pod.md) | 10 | Medium/Low | runnable (1) |

## H. Deny-side & admission (not synthesizable)

| Mechanism | Rows | Severity | Status |
|---|---|---|---|
| [`resourcequota-denies-creates.md`](resourcequota-denies-creates.md) | 33 | Low | not synthesizable — deny-side (woodpecker synthesizes permits) |
| [`validatingadmissionpolicy-denies-writes.md`](validatingadmissionpolicy-denies-writes.md) | 20 | Low | not synthesizable — deny-side (woodpecker synthesizes permits) |
| [`mutatingwebhook-intercepts.md`](mutatingwebhook-intercepts.md) | 3 | Critical | not synthesizable — deny-side / interception (woodpecker synthesizes permits) |
| [`crd-manages-customresources.md`](crd-manages-customresources.md) | 5 | High/Low | not synthesizable — no entity for (dynamic) custom resources in the schema |

**Total: 328 / 328 rows mapped.** Severity is the span of the paper's
`classification.txt` buckets across a file's rows.

