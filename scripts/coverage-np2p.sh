#!/bin/bash
# np2p crate coverage gate (staged threshold).
set -euo pipefail

THRESHOLD="${1:-${COVERAGE_NP2P:-47}}"
export PATH="${HOME}/.cargo/bin:${PATH}"
cd "$(dirname "$0")/.."

exec cargo llvm-cov -p np2p --fail-under-lines "${THRESHOLD}" -- --test-threads=4
