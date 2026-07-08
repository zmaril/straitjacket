#!/bin/sh
# Stand up the dev environment: wire up the committed git hooks, build the Rust
# crate, and install the docs site's dependencies. Run once after cloning:
#   ./scripts/dev.sh
set -e
run() { printf '\n\033[1m▶ %s\033[0m\n' "$*"; "$@"; }

# Route git at the committed hooks so the local CI gate runs before each commit.
run git config core.hooksPath .githooks

# Rust: build the CLI and pull the dependency tree.
run cargo build

# Docs site: install with the frozen lockfile, matching CI.
if command -v bun >/dev/null 2>&1; then
  run sh -c 'cd site && bun install --frozen-lockfile'
else
  printf '\n\033[1m⚠ bun not found — skipping site deps; install from https://bun.sh\033[0m\n'
fi

printf '\n\033[1m✓ dev environment ready\033[0m\n'
