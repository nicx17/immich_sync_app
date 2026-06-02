#!/bin/bash
set -e

# Change to the root of the repository to ensure cargo commands run properly
cd "$(dirname "$0")/.."

echo "Running cargo check..."
cargo check

echo ""
echo "Checking code formatting..."
if ! cargo fmt -- --check; then
    echo "Formatting issues found. Running 'cargo fmt' to fix them automatically..."
    cargo fmt
    echo "Formatting applied."
else
    echo "Formatting is correct."
fi
echo ""
echo "Running cargo clippy (with -D warnings)..."
cargo clippy --all-targets --all-features -- -D warnings

echo ""
echo "Running cargo audit..."
# Check if cargo-audit is installed, if not, offer a helpful message or install it
if ! cargo audit --version &> /dev/null; then
    echo "cargo-audit is not installed. Installing it now (this might take a moment)..."
    cargo install cargo-audit
fi
cargo audit

echo ""
echo "All checks passed successfully!"
