#!/bin/bash
# Backend coverage gate (staged thresholds).
#
# Measures the server application src/ (excluding runtime glue: lib.rs server
# assembly, telemetry.rs, src/bin, src/main.rs) plus the integration suites, and
# fails the deploy if the line-coverage % falls below the given threshold.
#
# Threshold is the first argument (default 40). Set COVERAGE_BACKEND to override.
#
# Note: this re-runs the full unit+integration suites under instrumentation, so
# it is the slowest part of the test gate. It needs the dev infra (./dev up) and
# the dev Postgres env (./dev test environment).

set -euo pipefail

THRESHOLD="${1:-${COVERAGE_BACKEND:-40}}"
export PATH="${HOME}/.cargo/bin:${PATH}"
cd "$(dirname "$0")/.."

export PGHOST=localhost PGPORT=25432 PGUSER=postgres PGPASSWORD=postgres

# Measure the library/unit + integration suites only. --all-targets also
# builds and runs the server/coordinator BIN targets, which try to open the
# websocket relay connection and can fail with close 1006 when the dev relay
# is already serving the running dev server. Bins are excluded from the metric
# below anyway, so --tests is both correct and avoids that failure mode.
exec cargo llvm-cov \
  --workspace \
  --tests \
  --fail-under-lines "${THRESHOLD}" \
  --ignore-filename-regex '(src/(lib|telemetry)\.rs)|(src/bin/)|(src/main\.rs)' \
  -- \
  --skip test_utils \
  --test-threads=8
