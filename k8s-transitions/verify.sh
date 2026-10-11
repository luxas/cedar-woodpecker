#!/usr/bin/env bash
# Validate every runnable transition in this folder with cedar-woodpecker.
# For each transitions/<name>.json that has a sibling transitions/<name>.cedar,
# run `cedar-woodpecker escalate` against k8s.cedarschema and assert that at
# least one escalation is reported and that every reported escalation is [sound].
#
# Usage: ./verify.sh            (build first: cargo build --release -p cedar-woodpecker)
#        WP=/path/to/cedar-woodpecker ./verify.sh
set -euo pipefail
cd "$(dirname "$0")"

SCHEMA=k8s.cedarschema
WP="${WP:-../target/release/cedar-woodpecker}"
if ! command -v "$WP" >/dev/null 2>&1 && [ ! -x "$WP" ]; then
  echo "cedar-woodpecker not found at '$WP'. Build it: cargo build --release -p cedar-woodpecker" >&2
  exit 2
fi
: "${CVC5:=$(command -v cvc5 || true)}"
export CVC5

fail=0 ok=0 skipped=0
for json in transitions/*.json; do
  name="$(basename "${json%.json}")"
  cedar="transitions/${name}.cedar"
  if [ ! -f "$cedar" ]; then
    skipped=$((skipped+1)); continue   # deny-side / NOT SYNTHESIZABLE
  fi
  out="$("$WP" escalate --schema "$SCHEMA" --policies "$cedar" --transitions "$json" 2>&1 || true)"
  if ! grep -q '^// ' <<<"$out"; then
    echo "FAIL  $name: no escalation reported"; echo "$out" | sed 's/^/      /'; fail=$((fail+1)); continue
  fi
  if grep -q '\[unsound\]\|NOT VALID' <<<"$out"; then
    echo "FAIL  $name: an escalation was unsound/invalid"; echo "$out" | sed 's/^/      /'; fail=$((fail+1)); continue
  fi
  echo "ok    $name"; ok=$((ok+1))
done

echo
echo "sound=$ok  failed=$fail  deny-side-skipped=$skipped"
[ "$fail" -eq 0 ]
