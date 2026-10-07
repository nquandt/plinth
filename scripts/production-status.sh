#!/usr/bin/env bash
# Counts the items of docs/PRODUCTION.md by gate and status (§16 Summary).
# Usage: bash scripts/production-status.sh [--open GATE]
#   --open GATE  also lists the items of GATE that are not Done.
set -euo pipefail
cd "$(dirname "$0")/.."
awk -F'|' -v want="${2:-}" '
  NF >= 6 && $2 ~ /^ [A-Z][0-9]+ $/ {
    id = $2; item = $3; g = $4; s = $5
    gsub(/ /, "", id); gsub(/ /, "", g); gsub(/ /, "", s); gsub(/^ +| +$/, "", item)
    count[g, s]++; gates[g] = 1
    if (want != "" && g == want && s != "Done") print id ": " s " - " item
  }
  END {
    if (want != "") print ""
    printf "%-8s %5s %8s %6s %7s\n", "Gate", "Done", "Partial", "Open", "Decide"
    split("Preview Beta 1.0 Later", order, " ")
    for (i = 1; i <= 4; i++) {
      g = order[i]
      if (g in gates) printf "%-8s %5d %8d %6d %7d\n", g, count[g, "Done"], count[g, "Partial"], count[g, "Open"], count[g, "Decide"]
    }
  }' docs/PRODUCTION.md
