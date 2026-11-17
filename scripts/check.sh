#!/usr/bin/env bash
# Все обязательные проверки из ТЗ — запускать непосредственно перед каждым коммитом.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
echo "ALL CHECKS PASSED"
