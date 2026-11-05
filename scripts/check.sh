#!/usr/bin/env bash
# Checks the orbe workspace. The engine has its own check, engine/scripts/check.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
