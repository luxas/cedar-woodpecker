#!/usr/bin/env bash
# Implicit permission: create pods/eviction -> delete Pod   (pod-eviction-deletes-pod)
#
# Proof: an identity granted ONLY `create pods/eviction` cannot `delete pods`,
# yet POSTing an Eviction makes the apiserver delete the Pod on its behalf.
# Mechanism: ../transitions/pod-eviction-deletes-pod.md
source "$(dirname "$0")/../lib.sh"
test_start "pod-eviction-deletes-pod"

ns="$(new_ns pod-evict)"

# Pre-existing state: a running victim Pod the attacker must not be able to delete.
step "admin creates a running victim Pod"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create victim pod"
apiVersion: v1
kind: Pod
metadata: { name: victim }
spec:
  automountServiceAccountToken: false
  containers:
  - name: c
    image: ${IMG}
    imagePullPolicy: IfNotPresent
    command: ["sh","-c","sleep 3600"]
EOF
wait_pod_running "$ns" victim 90

mk_sa "$ns" attacker
grant_ns "$ns" attacker "" "pods/eviction" "create"
grant_ns "$ns" attacker "" "pods" "get,list"      # observe only; NOT delete
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" create pods/eviction "$ns"
assert_cannot "$subj" delete pods          "$ns"

step "attacker POSTs an Eviction for the victim Pod"
as_subj "$subj" create --raw "/api/v1/namespaces/$ns/pods/victim/eviction" -f - >/dev/null <<EOF \
  || fail "attacker could not post the eviction"
{"apiVersion":"policy/v1","kind":"Eviction","metadata":{"name":"victim","namespace":"$ns"}}
EOF

# EFFECT: the apiserver eviction handler deletes the parent Pod.
wait_gone "$ns" pod victim 60
step "apiserver deleted the Pod via an Eviction the attacker posted with only 'create pods/eviction'"

test_pass
