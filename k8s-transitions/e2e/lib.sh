#!/usr/bin/env bash
# Shared harness for the implicit-permission end-to-end tests.
#
# Each test proves, on a live kind cluster, one implicit-permission transition
# `p -> q` from ../transitions/: it grants an identity ONLY the explicit
# permission p via RBAC, asserts the identity cannot do q directly (the GAP),
# performs p as that identity, and observes that Kubernetes performed q on its
# behalf (the EFFECT). The identity performs p ONLY; the harness, acting as the
# cluster admin, observes the effect (observing often itself needs q, which the
# identity by construction lacks).
#
# Contract for a test file (tests/<name>.sh):
#   source "$(dirname "$0")/../lib.sh"
#   test_start "<name>"
#   ... use helpers; assertions call fail() -> exit 1 on first failure ...
#   test_pass        # prints "RESULT: PASS <name>" and exits 0
# The runner (run.sh) treats exit 0 as PASS and anything else as FAIL.
set -uo pipefail

CTX="${WP_E2E_CONTEXT:-kind-wp-e2e}"
IMG="${WP_E2E_IMAGE:-busybox:1.36}"       # preloaded onto the node by run.sh/setup
RUN_ID="${RUN_ID:-$(date +%s)}"
_TESTNAME=""
_CLEANUP=()                                # resources to delete on exit, LIFO

K() { kubectl --context "$CTX" "$@"; }

# ---- logging ---------------------------------------------------------------
_c() { [ -t 1 ] && printf '\033[%sm' "$1" || true; }
log()     { printf '    %s\n' "$*"; }
step()    { printf '  %s %s\n' "$(_c 36)»$(_c 0)" "$*"; }
test_start() {
  _TESTNAME="$1"
  printf '%s== %s ==%s\n' "$(_c 1)" "$_TESTNAME" "$(_c 0)"
  trap _cleanup EXIT
}
fail()    { printf '  %sFAIL%s %s: %s\n' "$(_c 31)" "$(_c 0)" "$_TESTNAME" "$*" >&2; exit 1; }
test_pass() { printf '%sRESULT: PASS %s%s\n' "$(_c 32)" "$_TESTNAME" "$(_c 0)"; exit 0; }

# ---- cleanup ---------------------------------------------------------------
_track()  { _CLEANUP+=("$1"); }           # e.g. _track "namespace/foo"  _track "clusterrole/bar"
_cleanup() {
  local rc=$?
  for ((i=${#_CLEANUP[@]}-1; i>=0; i--)); do
    K delete "${_CLEANUP[$i]}" --ignore-not-found --wait=false >/dev/null 2>&1 || true
  done
  return $rc
}

# ---- identities & RBAC -----------------------------------------------------
rid() { echo "${1}-${RUN_ID}-$RANDOM"; }

new_ns() {                                 # new_ns [prefix] -> echoes namespace name
  local n; n="$(rid "${1:-wp}")"; n="$(tr '[:upper:]' '[:lower:]' <<<"$n")"
  K create namespace "$n" >/dev/null || fail "could not create namespace $n"
  _track "namespace/$n"; echo "$n"
}

mk_sa() {                                  # mk_sa NS NAME
  K -n "$1" create serviceaccount "$2" >/dev/null || fail "could not create sa $2"
}

sa_subject() { echo "system:serviceaccount:$1:$2"; }   # sa_subject NS NAME

# grant_ns NS SA APIGROUP RESOURCES VERBS
#   APIGROUP "" for core; RESOURCES/VERBS comma-separated; RESOURCES may name a
#   subresource ("pods/exec"). Creates a namespaced Role + RoleBinding to the SA.
grant_ns() {
  local ns=$1 sa=$2 grp=$3 res=$4 verbs=$5 role; role="$(rid role)"
  _yaml_rule "Role" "$ns" "$role" "$grp" "$res" "$verbs" | K apply -f - >/dev/null \
    || fail "could not create Role $role"
  _yaml_binding "RoleBinding" "$ns" "$role" "$ns" "$sa" | K apply -f - >/dev/null \
    || fail "could not create RoleBinding for $role"
}

# grant_cluster SA_NS SA APIGROUP RESOURCES VERBS  -> cluster-wide ClusterRole+CRB
grant_cluster() {
  local ns=$1 sa=$2 grp=$3 res=$4 verbs=$5 role; role="$(rid crole)"
  _yaml_rule "ClusterRole" "" "$role" "$grp" "$res" "$verbs" | K apply -f - >/dev/null \
    || fail "could not create ClusterRole $role"
  _yaml_binding "ClusterRoleBinding" "" "$role" "$ns" "$sa" | K apply -f - >/dev/null \
    || fail "could not create ClusterRoleBinding for $role"
  _track "clusterrole/$role"
  _track "clusterrolebinding/${role}-crb"
}

_yaml_rule() {   # KIND NS NAME APIGROUP RES VERBS
  local kind=$1 ns=$2 name=$3 grp=$4 res=$5 verbs=$6
  echo "apiVersion: rbac.authorization.k8s.io/v1"
  echo "kind: $kind"
  echo "metadata:"
  echo "  name: $name"
  [ -n "$ns" ] && echo "  namespace: $ns"
  echo "rules:"
  echo "- apiGroups: [\"$grp\"]"
  echo -n "  resources: ["; _csv_quote "$res"; echo "]"
  echo -n "  verbs: ["; _csv_quote "$verbs"; echo "]"
}
_yaml_binding() {  # KIND NS ROLENAME SA_NS SA
  local kind=$1 ns=$2 role=$3 sans=$4 sa=$5 bname
  if [ "$kind" = "ClusterRoleBinding" ]; then bname="${role}-crb"; else bname="${role}-rb"; fi
  echo "apiVersion: rbac.authorization.k8s.io/v1"
  echo "kind: $kind"
  echo "metadata:"
  echo "  name: $bname"
  [ -n "$ns" ] && echo "  namespace: $ns"
  echo "roleRef:"
  echo "  apiGroup: rbac.authorization.k8s.io"
  echo "  kind: ${kind/Binding/}"
  echo "  name: $role"
  echo "subjects:"
  echo "- kind: ServiceAccount"
  echo "  name: $sa"
  echo "  namespace: $sans"
}
_csv_quote() { local IFS=, first=1 x; for x in $1; do [ $first -eq 1 ] || echo -n ", "; echo -n "\"$x\""; first=0; done; }

# ---- acting as an identity -------------------------------------------------
# as_subj SUBJECT kubectl-args...   (impersonation; RBAC is evaluated as SUBJECT)
as_subj() { local s=$1; shift; K --as="$s" "$@"; }

# ---- assertions ------------------------------------------------------------
# _cani SUBJECT VERB RESOURCE [NS]  -> prints yes/no.
# RESOURCE may be "pods/exec": `kubectl auth can-i` needs the subresource via
# the --subresource flag (the "resource/subresource" slash form silently reads
# as the bare resource and gives a wrong answer), while RBAC *rules* use the
# slash form -- so this split applies only to the query, not to grant_ns.
_cani() {
  local s=$1 verb=$2 res=$3 ns=${4:-} args=(auth can-i "$2")
  if [[ "$res" == */* ]]; then args+=("${res%%/*}" "--subresource=${res#*/}"); else args+=("$res"); fi
  args+=("--as=$s"); [ -n "$ns" ] && args+=(-n "$ns")
  K "${args[@]}" 2>/dev/null || true
}
# assert_cannot SUBJECT VERB RESOURCE [NS]   -- the GAP: SUBJECT must NOT hold q
assert_cannot() {
  local out; out="$(_cani "$1" "$2" "$3" "${4:-}")"
  [ "$out" = "no" ] || fail "GAP not clean: '$1' CAN '$2 $3'${4:+ in $4} (auth can-i => '$out', expected 'no')"
  step "gap confirmed: $1 cannot '$2 $3'${4:+ in $4}"
}
# assert_can SUBJECT VERB RESOURCE [NS]      -- sanity: SUBJECT holds p
assert_can() {
  local out; out="$(_cani "$1" "$2" "$3" "${4:-}")"
  [ "$out" = "yes" ] || fail "'$1' lacks the explicit permission '$2 $3'${4:+ in $4} (auth can-i => '$out')"
  step "explicit permission held: $1 can '$2 $3'${4:+ in $4}"
}
assert_eq()        { [ "$1" = "$2" ] || fail "${3:-assert_eq}: '$1' != '$2'"; }
assert_contains()  { case "$1" in *"$2"*) :;; *) fail "${3:-assert_contains}: '$1' does not contain '$2'";; esac; }
assert_nonempty()  { [ -n "$1" ] || fail "${2:-assert_nonempty}: value was empty"; }

# wait_pod_running NS POD [timeout]
wait_pod_running() {
  local ns=$1 pod=$2 to=${3:-90}
  K -n "$ns" wait --for=condition=Ready "pod/$pod" --timeout="${to}s" >/dev/null 2>&1 \
    || K -n "$ns" get "pod/$pod" -o jsonpath='{.status.phase}' | grep -q Running \
    || fail "pod $ns/$pod did not reach Running within ${to}s ($(K -n "$ns" get pod/$pod -o jsonpath='{.status.phase}:{.status.containerStatuses[*].state}' 2>/dev/null))"
}

# wait_gone NS KIND NAME [timeout]  -- assert an object is deleted
wait_gone() {
  local ns=$1 kind=$2 name=$3 to=${4:-60} i=0
  while [ $i -lt "$to" ]; do
    K -n "$ns" get "$kind" "$name" >/dev/null 2>&1 || { step "$kind $ns/$name is gone"; return 0; }
    sleep 1; i=$((i+1))
  done
  fail "$kind $ns/$name still exists after ${to}s"
}

# wait_exists NS KIND SELECTOR_OR_NAME [timeout] -- assert an object appears
wait_exists() {
  local ns=$1 kind=$2 sel=$3 to=${4:-60} i=0 n
  while [ $i -lt "$to" ]; do
    if [[ "$sel" == *=* ]]; then
      n="$(K -n "$ns" get "$kind" -l "$sel" --no-headers 2>/dev/null | wc -l)"
    else
      n="$(K -n "$ns" get "$kind" "$sel" --no-headers 2>/dev/null | wc -l)"
    fi
    [ "${n:-0}" -ge 1 ] && { step "$kind matching '$sel' exists in $ns ($n)"; return 0; }
    sleep 1; i=$((i+1))
  done
  fail "no $kind matching '$sel' appeared in $ns within ${to}s"
}
