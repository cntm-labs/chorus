#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HOOKS_DIR="$REPO_ROOT/.git/hooks"

echo "Installing git hooks..."
ln -sf "$SCRIPT_DIR/pre-commit" "$HOOKS_DIR/pre-commit"
echo "Done. Pre-commit hook installed."
