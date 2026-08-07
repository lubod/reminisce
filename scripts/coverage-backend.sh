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

exec cargo llvm-cov \
  --workspace \
  --all-targets \
  --fail-under-lines "${THRESHOLD}" \
  --ignore-filename-regex '(src/(lib|telemetry)\.rs)|(src/bin/)|(src/main\.rs)' \
  -- \
  --skip test_utils \
  --test-threads=8
