#!/usr/bin/env bash
# Implicit permission: create FlowSchema -> get PriorityLevelConfiguration
#                                           (flowschema-references-prioritylevel)
#
# Proof: an identity granted ONLY `create flowschemas` (cluster-scoped APF
# config) cannot `get prioritylevelconfigurations`, yet when it creates a
# FlowSchema that references a PriorityLevelConfiguration by name
# (spec.priorityLevelConfiguration.name), the apiserver's APF config controller
# READS the referenced PriorityLevelConfiguration on the identity's behalf to
# wire the schema to its concurrency pool. We observe that read operationally
# via the FlowSchema's `Dangling` status condition, which the APF controller
# sets from the result of resolving the referenced PLC:
#   references a NON-existent PLC  -> Dangling=True  reason=NotFound
#   references an EXISTING   PLC  -> Dangling=False reason=Found
# The flip from NotFound to Found is the controller's get of the named PLC made
# visible. Explicit p = "create a FlowSchema"; implicit q = "cause APF to get
# the referenced PriorityLevelConfiguration".
# Spec: ../transitions/flowschema-references-prioritylevel.{md,json}
#   when: resource.name == context.resource1.priorityLevelName
source "$(dirname "$0")/../lib.sh"
test_start "flowschema-references-prioritylevel"

# Local cleanup is defensive redundancy: lib.sh uses a FILE-backed cleanup
# ledger (WP_CLEANUP_FILE) that survives the $(...) subshell of new_ns /
# grant_cluster, so the harness _cleanup already deletes tracked objects on
# exit. We additionally track our own objects here (and, for cluster-scoped
# tests, create them with names we control) and chain our trap ahead of
# lib's _cleanup.
_LOCAL_CLEAN=()
_lclean() { local rc=$?; for ((i=${#_LOCAL_CLEAN[@]}-1; i>=0; i--)); do
  K delete "${_LOCAL_CLEAN[$i]}" --ignore-not-found --wait=false >/dev/null 2>&1 || true; done; return $rc; }
trap '_lclean; _cleanup' EXIT

home_ns="$(new_ns fs-plc)"; _LOCAL_CLEAN+=("namespace/$home_ns")
mk_sa "$home_ns" attacker
subj="$(sa_subject "$home_ns" attacker)"
subj_grp="flowcontrol.apiserver.k8s.io"

# Local cleanup is defensive redundancy: lib.sh uses a FILE-backed cleanup
# ledger (WP_CLEANUP_FILE) that survives the $(...) subshell of new_ns /
# grant_cluster, so the harness _cleanup already deletes tracked objects on
# exit. We additionally track our own objects here (and, for cluster-scoped
# tests, create them with names we control) and chain our trap ahead of
# lib's _cleanup.
crole="$(rid wp-fs-crole)"; crb="${crole}-crb"
_LOCAL_CLEAN+=("clusterrole/$crole" "clusterrolebinding/$crb")
K apply -f - >/dev/null <<EOF || fail "could not create ClusterRole $crole"
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata: { name: $crole }
rules:
- apiGroups: ["$subj_grp"]
  resources: ["flowschemas"]
  verbs: ["create","get","list"]
EOF
K apply -f - >/dev/null <<EOF || fail "could not create ClusterRoleBinding $crb"
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata: { name: $crb }
roleRef: { apiGroup: rbac.authorization.k8s.io, kind: ClusterRole, name: $crole }
subjects:
- { kind: ServiceAccount, name: attacker, namespace: $home_ns }
EOF

# THE GAP: holds p (create flowschemas) but not q (get prioritylevelconfigurations).
assert_can    "$subj" create flowschemas.$subj_grp
assert_cannot "$subj" get    prioritylevelconfigurations.$subj_grp

# Two unique names so concurrent runs never collide; both cluster-scoped, both
# tracked for cleanup. 'global-default' is a built-in PLC that always exists.
missing_plc="wp-no-such-plc-$RUN_ID-$RANDOM"
real_plc="global-default"
fs="$(rid wp-fs)";  _LOCAL_CLEAN+=("flowschema.$subj_grp/$fs")
fs2="$(rid wp-fs)"; _LOCAL_CLEAN+=("flowschema.$subj_grp/$fs2")

# ACT (as attacker, exercising ONLY p): create a FlowSchema that references a
# PLC that does NOT exist. matchingPrecedence and a unique, non-matching subject
# group keep this schema inert (it matches no real traffic).
step "attacker creates FlowSchema/$fs referencing a non-existent PLC '$missing_plc'"
as_subj "$subj" apply -f - >/dev/null <<EOF || fail "attacker could not create the flowschema"
apiVersion: flowcontrol.apiserver.k8s.io/v1
kind: FlowSchema
metadata: { name: ${fs} }
spec:
  priorityLevelConfiguration: { name: ${missing_plc} }
  matchingPrecedence: 9999
  rules:
  - subjects: [{ kind: Group, group: { name: wp-nonexistent-group-${RUN_ID} } }]
    resourceRules:
    - verbs: ["get"]
      apiGroups: [""]
      resources: ["configmaps"]
      namespaces: ["wp-nonexistent-ns-${RUN_ID}"]
EOF

# OBSERVE (as admin) that the APF controller resolved the referenced PLC and,
# finding it absent, marked the schema Dangling=True reason=NotFound. This is
# the controller performing the get on the attacker's behalf -- the attacker
# cannot itself get PLCs.
_wait_dangling() {   # NS(unused) FS EXPECT_STATUS EXPECT_REASON [timeout]
  local fsn=$1 want_status=$2 want_reason=$3 to=${4:-30} i=0 st rs
  while [ $i -lt "$to" ]; do
    st="$(K get flowschema "$fsn" -o jsonpath='{.status.conditions[?(@.type=="Dangling")].status}' 2>/dev/null)"
    rs="$(K get flowschema "$fsn" -o jsonpath='{.status.conditions[?(@.type=="Dangling")].reason}'  2>/dev/null)"
    [ "$st" = "$want_status" ] && [ "$rs" = "$want_reason" ] && { echo "$st/$rs"; return 0; }
    sleep 1; i=$((i+1))
  done
  echo "${st:-<none>}/${rs:-<none>}"; return 1
}

got="$(_wait_dangling "$fs" "True" "NotFound" 30)" \
  || fail "APF never marked FlowSchema Dangling=True/NotFound for the missing PLC (saw $got)"
step "APF resolved the referenced PLC '$missing_plc' and found it ABSENT -> Dangling=$got (controller did the get on the attacker's behalf)"

# Now, STILL exercising ONLY p (create), author a SECOND FlowSchema that
# references a PLC which DOES exist. The APF controller resolves the referenced
# PLC and, finding it present, sets Dangling=False/Found -- the complementary
# outcome of the same get, driven purely by the create path (no update/patch).
step "attacker creates FlowSchema/$fs2 referencing an existing PLC '$real_plc'"
as_subj "$subj" apply -f - >/dev/null <<EOF || fail "attacker could not create the second flowschema"
apiVersion: flowcontrol.apiserver.k8s.io/v1
kind: FlowSchema
metadata: { name: ${fs2} }
spec:
  priorityLevelConfiguration: { name: ${real_plc} }
  matchingPrecedence: 9998
  rules:
  - subjects: [{ kind: Group, group: { name: wp-nonexistent-group2-${RUN_ID} } }]
    resourceRules:
    - verbs: ["get"]
      apiGroups: [""]
      resources: ["configmaps"]
      namespaces: ["wp-nonexistent-ns2-${RUN_ID}"]
EOF

got="$(_wait_dangling "$fs2" "False" "Found" 30)" \
  || fail "APF never set Dangling=False/Found for a FlowSchema referencing an existing PLC (saw $got)"
step "APF resolved the referenced PLC '$real_plc' and found it PRESENT -> Dangling=$got; the apiserver got the PLC the attacker was never granted 'get' on"

test_pass
