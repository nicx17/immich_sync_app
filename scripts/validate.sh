#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."

step() { printf '\n\033[1;34m>> %s\033[0m\n' "$1"; }

step "Checking code formatting..."
if ! cargo fmt --all -- --check; then
    echo "Formatting issues found. Running 'cargo fmt' to fix them automatically..."
    cargo fmt --all
    echo "Formatting applied."
fi

step "Running cargo clippy (with -D warnings)..."
cargo clippy --locked --all-targets --all-features -- -D warnings

step "Running tests..."
cargo test --locked

step "Running cargo audit..."
if ! command -v cargo-audit &>/dev/null; then
    echo "cargo-audit not found. Install with: cargo install cargo-audit"
    exit 1
fi
cargo audit --deny warnings

echo ""
echo "All checks passed successfully!"
