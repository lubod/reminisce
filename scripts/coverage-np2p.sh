#!/bin/bash
# np2p crate coverage gate (staged threshold).
#
# Measures the np2p library + integration tests. The daemon binary (bin/main.rs)
# is excluded from the metric — it is runtime glue that tries to open the live
# coordinator websocket relay and is not unit-testable, mirroring how the
# backend gate excludes its bin targets.
set -euo pipefail

THRESHOLD="${1:-${COVERAGE_NP2P:-70}}"
export PATH="${HOME}/.cargo/bin:${PATH}"
cd "$(dirname "$0")/.."

exec cargo llvm-cov \
  -p np2p \
  --fail-under-lines "${THRESHOLD}" \
  --ignore-filename-regex '(bin/main\.rs)' \
  -- \
  --test-threads=4