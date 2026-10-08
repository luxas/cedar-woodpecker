#!/usr/bin/env bash
# Run the implicit-permission end-to-end tests against the kind cluster.
#
#   ./run.sh                 # run every tests/*.sh
#   ./run.sh a.sh b.sh       # run only the named tests
#   WP_E2E_CONTEXT=... ./run.sh
#
# Each test is a standalone script: exit 0 => PASS, non-zero => FAIL. A test may
# print "RESULT: SKIP <name> <reason>" and exit 3 to self-skip (e.g. an API not
# present in this cluster). Per-test logs land in results/, with a summary.json
# and a printed table. Exit status is non-zero iff any test FAILED.
set -uo pipefail
cd "$(dirname "$0")"
export PATH="$HOME/.local/bin:$PATH"
CTX="${WP_E2E_CONTEXT:-kind-wp-e2e}"
export WP_E2E_CONTEXT="$CTX"

kubectl --context "$CTX" version >/dev/null 2>&1 || {
  echo "cluster context '$CTX' not reachable. Run ./setup.sh first." >&2; exit 2; }

mkdir -p results
if [ $# -gt 0 ]; then
  TESTS=(); for a in "$@"; do TESTS+=("tests/$(basename "$a")"); done
else
  mapfile -t TESTS < <(ls tests/*.sh | sort)
fi

pass=0 fail=0 skip=0; declare -a rows
for t in "${TESTS[@]}"; do
  name="$(basename "${t%.sh}")"
  log="results/${name}.log"
  RUN_ID="$(date +%s)" bash "$t" >"$log" 2>&1; rc=$?
  if [ $rc -eq 0 ]; then
    pass=$((pass+1)); rows+=("PASS  $name"); printf 'PASS  %s\n' "$name"
  elif [ $rc -eq 3 ]; then
    reason="$(grep -m1 '^RESULT: SKIP' "$log" | sed 's/^RESULT: SKIP [^ ]* //')"
    skip=$((skip+1)); rows+=("SKIP  $name  ${reason}"); printf 'SKIP  %s  %s\n' "$name" "$reason"
  else
    fail=$((fail+1)); rows+=("FAIL  $name"); printf 'FAIL  %s  (see %s)\n' "$name" "$log"
    tail -n 4 "$log" | sed 's/^/        /'
  fi
done

total=${#TESTS[@]}
{
  echo "{"
  echo "  \"context\": \"$CTX\","
  echo "  \"total\": $total, \"pass\": $pass, \"fail\": $fail, \"skip\": $skip,"
  echo "  \"results\": ["
  n=0; for r in "${rows[@]}"; do
    st="${r%%  *}"; rest="${r#*  }"; nm="${rest%%  *}"
    n=$((n+1)); comma=,; [ $n -eq ${#rows[@]} ] && comma=
    printf '    {"name": "%s", "status": "%s"}%s\n' "$nm" "$st" "$comma"
  done
  echo "  ]"
  echo "}"
} > results/summary.json

echo
echo "==================== SUMMARY ===================="
printf 'total=%d  pass=%d  fail=%d  skip=%d\n' "$total" "$pass" "$fail" "$skip"
echo "summary: results/summary.json"
[ "$fail" -eq 0 ]
