#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOOKS_DIR="$(git rev-parse --git-common-dir)/hooks"

echo "Installing git hooks..."
mkdir -p "$HOOKS_DIR"
ln -sf "$SCRIPT_DIR/pre-commit" "$HOOKS_DIR/pre-commit"
echo "Done. Pre-commit hook installed."
