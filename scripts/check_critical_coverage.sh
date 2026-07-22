#!/bin/sh
set -eu

report=${1:?usage: check_critical_coverage.sh <lcov-report>}
minimum=${2:-95}

awk -v minimum="$minimum" '
BEGIN {
    targets["crates/compass-parity/src/lib.rs"] = "compatibility"
    targets["crates/compass-model/src/graph.rs"] = "serialization"
    targets["crates/compass-files/src/cache.rs"] = "cache"
    targets["crates/compass-files/src/build_guard.rs"] = "security-atomic"
    targets["crates/compass-core/src/raw_guard.rs"] = "security-shrink"
}
/^SF:/ {
    current = substr($0, 4)
    selected = ""
    for (target in targets) {
        if (length(current) >= length(target) && substr(current, length(current) - length(target) + 1) == target) {
            selected = target
            seen[target] = 1
            break
        }
    }
    next
}
/^LF:/ && selected != "" { total[selected] = substr($0, 4) + 0; next }
/^LH:/ && selected != "" { hit[selected] = substr($0, 4) + 0; next }
END {
    failed = 0
    for (target in targets) {
        if (!seen[target] || total[target] == 0) {
            printf "critical coverage missing: %s (%s)\n", targets[target], target > "/dev/stderr"
            failed = 1
            continue
        }
        coverage = 100 * hit[target] / total[target]
        printf "critical coverage: %-13s %6.2f%% (%d/%d lines)\n", targets[target], coverage, hit[target], total[target]
        if (coverage + 0.000001 < minimum) {
            printf "critical coverage below %.2f%%: %s\n", minimum, target > "/dev/stderr"
            failed = 1
        }
    }
    exit failed
}
' "$report"
