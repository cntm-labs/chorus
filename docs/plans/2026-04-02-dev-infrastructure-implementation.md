# Development Infrastructure Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Set up production-grade dev infrastructure for Chorus CPaaS — tooling configs, CI/CD, coverage, pre-commit hooks, CLAUDE.md guard rails.

**Architecture:** Add config files at workspace root, GitHub Actions workflows under `.github/workflows/`, update `Cargo.toml` lints, and enhance `CLAUDE.md` with AI development guard rails and design pattern documentation.

**Tech Stack:** Rust (stable), GitHub Actions, cargo-deny, cargo-nextest, cargo-llvm-cov, Codecov, bacon, Claude Code Action, Release Please, Docker (ghcr.io)

**Execution order:** Tooling configs → Workspace lints → CI workflows → Claude/Release workflows → CLAUDE.md → Pre-commit hooks → Verify all

---

## Task 1: Rust Toolchain & Formatter Config

### Files
- Create: `rust-toolchain.toml`
- Create: `rustfmt.toml`
- Create: `.clippy.toml`

### Step 1: Create rust-toolchain.toml

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy", "llvm-tools-preview"]
```

### Step 2: Create rustfmt.toml

```toml
edition = "2021"
max_width = 100
use_field_init_shorthand = true
group_imports = "StdExternalCrate"
imports_granularity = "Crate"
```

### Step 3: Create .clippy.toml

```toml
msrv = "1.85.0"
disallowed-names = ["foo", "bar", "baz", "tmp", "temp"]
too-many-arguments-threshold = 10
```

### Step 4: Run cargo fmt and verify

Run: `cargo fmt --all`
Then: `cargo fmt --all -- --check`
Expected: No formatting issues (files may be reformatted by new config)

### Step 5: Commit

```bash
git add rust-toolchain.toml rustfmt.toml .clippy.toml
git commit -m "chore: add rust-toolchain.toml, rustfmt.toml, .clippy.toml"
```

---

## Task 2: Cargo Aliases & Bacon

### Files
- Create: `.cargo/config.toml`
- Create: `bacon.toml`

### Step 1: Create .cargo/config.toml

```toml
[alias]
check-all = "check --workspace --all-targets"
test-all = "test --workspace"
lint = "clippy --workspace --all-targets -- -D warnings"
```

### Step 2: Create bacon.toml

```toml
default_job = "check"

[jobs.check]
command = ["cargo", "check", "--workspace", "--color", "always"]
watch = ["crates"]

[jobs.clippy]
command = ["cargo", "clippy", "--workspace", "--all-targets", "--color", "always", "--", "-D", "warnings"]
watch = ["crates"]

[jobs.test]
command = ["cargo", "test", "--workspace", "--color", "always"]
watch = ["crates"]

[jobs.nextest]
command = ["cargo", "nextest", "run", "--workspace", "--color", "always"]
watch = ["crates"]

[jobs.fmt]
command = ["cargo", "fmt", "--all"]
watch = ["crates"]

[keybindings]
c = "job:clippy"
t = "job:test"
n = "job:nextest"
f = "job:fmt"
```

### Step 3: Verify aliases work

Run: `cargo check-all`
Expected: Compiles with no errors

Run: `cargo lint`
Expected: No warnings

### Step 4: Commit

```bash
git add .cargo/config.toml bacon.toml
git commit -m "chore: add cargo aliases and bacon.toml for dev watch"
```

---

## Task 3: cargo-deny Config

### Files
- Create: `deny.toml`

### Step 1: Create deny.toml

```toml
[graph]
targets = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
]
all-features = true

[advisories]
db-path = "~/.cargo/advisory-db"
db-urls = ["https://github.com/rustsec/advisory-db"]

[licenses]
private = { ignore = true }
allow = [
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-3.0",
    "BSL-1.0",
    "0BSD",
    "Zlib",
]
confidence-threshold = 0.8

[bans]
multiple-versions = "warn"
wildcards = "deny"
highlight = "simplest-path"

[sources]
unknown-registry = "deny"
unknown-git = "warn"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

### Step 2: Run cargo deny (if installed)

Run: `cargo deny check 2>&1 || echo "cargo-deny not installed locally, will run in CI"`
Expected: Either passes or reports cargo-deny not installed

### Step 3: Commit

```bash
git add deny.toml
git commit -m "chore: add deny.toml for dependency auditing"
```

---

## Task 4: Codecov Config

### Files
- Create: `codecov.yml`

### Step 1: Create codecov.yml

```yaml
codecov:
  require_ci_to_pass: true

coverage:
  status:
    project:
      default:
        target: auto
        threshold: 5%
      core:
        target: auto
        threshold: 5%
        flags: [core]
      providers:
        target: auto
        threshold: 5%
        flags: [providers]
    patch:
      default:
        target: 70%

flags:
  core:
    paths: [crates/chorus-core/]
    carryforward: true
  providers:
    paths: [crates/chorus-providers/]
    carryforward: true

ignore:
  - "sdks/"
  - "load-tests/"
  - "deploy/"
  - "crates/*/benches/"

comment:
  layout: "reach,diff,flags,files"
  behavior: default
  require_changes: true
```

### Step 2: Commit

```bash
git add codecov.yml
git commit -m "chore: add codecov.yml with per-crate coverage flags"
```

---

## Task 5: Update Workspace Lints

### Files
- Modify: `Cargo.toml` — workspace lints section

### Step 1: Update workspace lints in Cargo.toml

Replace the existing `[workspace.lints.rust]` and `[workspace.lints.clippy]` sections:

```toml
[workspace.lints.rust]
unsafe_code = "forbid"
dead_code = "deny"
unused_imports = "deny"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
module_name_repetitions = "allow"
must_use_candidate = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"
dbg_macro = "deny"
todo = "warn"
print_stdout = "warn"
print_stderr = "warn"
```

### Step 2: Add release profile

Add at the end of Cargo.toml:

```toml
[profile.release]
strip = true
lto = true
codegen-units = 1
```

### Step 3: Verify lint changes compile

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: No errors (may have new warnings to fix)

### Step 4: Fix any lint issues that arise

Fix any `dead_code`, `unused_imports`, `dbg_macro` violations.

### Step 5: Commit

```bash
git commit -am "chore: strengthen workspace lints (deny dead_code, unused_imports, dbg_macro)"
```

---

## Task 6: CI Workflow — Core (fmt, clippy, test, deny)

### Files
- Create: `.github/workflows/ci.yml`

### Step 1: Create .github/workflows/ci.yml

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always

jobs:
  fmt:
    name: Format
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --all -- --check

  clippy:
    name: Clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --workspace --all-targets -- -D warnings

  test:
    name: Test & Coverage
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: llvm-tools-preview
      - uses: Swatinem/rust-cache@v2
      - uses: taiki-e/install-action@cargo-llvm-cov
      - uses: taiki-e/install-action@nextest

      - name: Run tests with coverage
        run: cargo llvm-cov nextest --workspace --lcov --output-path lcov.info

      - name: Generate HTML coverage report
        run: cargo llvm-cov nextest --workspace --html --output-dir coverage-html

      - name: Upload coverage to Codecov
        uses: codecov/codecov-action@v5
        with:
          files: lcov.info
          fail_ci_if_error: false
          token: ${{ secrets.CODECOV_TOKEN }}

      - name: Upload HTML coverage report
        uses: actions/upload-artifact@v4
        with:
          name: coverage-report
          path: coverage-html/
          retention-days: 30

  deny:
    name: Dependency Check
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: EmbarkStudios/cargo-deny-action@v2
```

### Step 2: Commit

```bash
mkdir -p .github/workflows
git add .github/workflows/ci.yml
git commit -m "ci: add core CI workflow (fmt, clippy, test+coverage, deny)"
```

---

## Task 7: Security Workflow

### Files
- Create: `.github/workflows/security.yml`

### Step 1: Create .github/workflows/security.yml

```yaml
name: Security

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  cargo-audit:
    name: Audit
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo install cargo-audit --locked
      - run: cargo audit

  cargo-deny:
    name: Deny
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: EmbarkStudios/cargo-deny-action@v2
```

### Step 2: Commit

```bash
git add .github/workflows/security.yml
git commit -m "ci: add security workflow (cargo-audit, cargo-deny)"
```

---

## Task 8: Claude Code Action Workflows

### Files
- Create: `.github/workflows/claude.yml`
- Create: `.github/workflows/weekly-digest.yml`

### Step 1: Create .github/workflows/claude.yml

```yaml
name: Claude Code

on:
  issue_comment:
    types: [created]
  pull_request_review_comment:
    types: [created]
  issues:
    types: [opened, assigned]
  pull_request:
    types: [opened, synchronize, reopened]
  pull_request_review:
    types: [submitted]

concurrency:
  group: claude-${{ github.event.issue.number || github.event.pull_request.number }}
  cancel-in-progress: false

jobs:
  claude:
    if: |
      (github.event_name == 'pull_request') ||
      (github.event_name == 'issue_comment' && contains(github.event.comment.body, '@claude')) ||
      (github.event_name == 'pull_request_review_comment' && contains(github.event.comment.body, '@claude')) ||
      (github.event_name == 'pull_request_review' && contains(github.event.review.body, '@claude')) ||
      (github.event_name == 'issues' && contains(github.event.issue.title, '@claude'))
    runs-on: ubuntu-latest
    timeout-minutes: 30
    permissions:
      contents: write
      issues: write
      pull-requests: write
      id-token: write
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt

      - uses: Swatinem/rust-cache@v2

      - uses: anthropics/claude-code-action@v1
        with:
          claude_code_oauth_token: ${{ secrets.CLAUDE_CODE_OAUTH_TOKEN }}
          trigger_phrase: "@claude"
          assignee_trigger: "claude"
          prompt: |
            You are working on Chorus — an open-source CPaaS (Communications Platform as a Service)
            built in Rust. It provides SMS and Email delivery with smart routing, multi-provider
            failover, and cost optimization.

            Key commands:
              cargo check --workspace
              cargo test --workspace
              cargo clippy --workspace -- -D warnings
              cargo fmt --all

            Read CLAUDE.md for full project context before making changes.
            Always run tests before marking work as done.

            For PR reviews, analyze:
            - Security issues (credential handling, injection)
            - Breaking API changes
            - Missing tests for critical paths
            - Dependency rule violations (chorus-core is a leaf crate)
```

### Step 2: Create .github/workflows/weekly-digest.yml

```yaml
name: Weekly Digest

on:
  schedule:
    - cron: '0 9 * * 1'

jobs:
  digest:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      issues: write
      id-token: write
    steps:
      - uses: actions/checkout@v4
      - uses: anthropics/claude-code-action@v1
        with:
          claude_code_oauth_token: ${{ secrets.CLAUDE_CODE_OAUTH_TOKEN }}
          direct_prompt: |
            Generate a weekly engineering digest for Chorus CPaaS:
            - Test suite status and coverage trends
            - New dependencies added this week
            - Open issues summary
            - Security advisory updates
            - Action items for next week
```

### Step 3: Commit

```bash
git add .github/workflows/claude.yml .github/workflows/weekly-digest.yml
git commit -m "ci: add Claude Code Action and weekly digest workflows"
```

---

## Task 9: Release Please & Docker Workflow

### Files
- Create: `release-please-config.json`
- Create: `.release-please-manifest.json`
- Create: `.github/workflows/release-please.yml`

### Step 1: Create release-please-config.json

```json
{
  "packages": {
    ".": {
      "release-type": "rust",
      "bump-minor-pre-major": true,
      "bump-patch-for-minor-pre-major": true
    }
  }
}
```

### Step 2: Create .release-please-manifest.json

```json
{
  ".": "0.1.0"
}
```

### Step 3: Create .github/workflows/release-please.yml

```yaml
name: Release Please

on:
  push:
    branches: [main]

permissions:
  contents: write
  pull-requests: write

jobs:
  release-please:
    runs-on: ubuntu-latest
    outputs:
      releases_created: ${{ steps.release.outputs.releases_created }}
      tag_name: ${{ steps.release.outputs.tag_name }}
    steps:
      - uses: googleapis/release-please-action@v4
        id: release
        with:
          token: ${{ secrets.GITHUB_TOKEN }}
          config-file: release-please-config.json
          manifest-file: .release-please-manifest.json

  ci-gate:
    needs: release-please
    if: needs.release-please.outputs.releases_created == 'true'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all -- --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace

  docker:
    needs: [release-please, ci-gate]
    if: needs.release-please.outputs.releases_created == 'true'
    runs-on: ubuntu-latest
    permissions:
      contents: read
      packages: write
    steps:
      - uses: actions/checkout@v4
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
      - uses: docker/build-push-action@v5
        with:
          context: .
          push: true
          tags: |
            ghcr.io/${{ github.repository }}:${{ needs.release-please.outputs.tag_name }}
            ghcr.io/${{ github.repository }}:latest
```

### Step 4: Commit

```bash
git add release-please-config.json .release-please-manifest.json .github/workflows/release-please.yml
git commit -m "ci: add Release Please with Docker build on release"
```

---

## Task 10: Update CLAUDE.md

### Files
- Modify: `CLAUDE.md`

### Step 1: Add new sections to CLAUDE.md

Add the following sections after the existing "Conventions" section:

```markdown
## Build Commands
\`\`\`sh
cargo check --workspace          # Type check
cargo test --workspace           # Run all tests
cargo clippy --workspace -- -D warnings  # Lint
cargo fmt --all                  # Format
cargo deny check                 # License + advisory check
cargo llvm-cov nextest --workspace  # Test with coverage
\`\`\`

## AI Development Guard Rails

### Anti-patterns to Avoid
- No `#[allow(dead_code)]` — use it or remove it
- No duplicate functions/structs — extract shared logic to chorus-core
- No spaghetti dependencies — follow dependency rules strictly
- No magic numbers — use named constants
- Max file ~300 lines — split if exceeding
- No `dbg!()`, `print!()`, `todo!()` in production code
- All public types/functions must have doc comments

### Design Patterns Used in Chorus

| Pattern | Where | Why |
|---------|-------|-----|
| Builder | `Chorus::builder()` | Complex config step-by-step |
| Strategy | `SmsSender`, `EmailSender` traits | Swap providers at runtime |
| Chain of Responsibility | `WaterfallRouter` | Try providers sequentially, fallback on failure |
| Facade | `Chorus` client | Hide router/template/provider complexity |
| Adapter | Provider implementations | Normalize different APIs to common trait |
| Template Method | `Template::render()` | Algorithm skeleton for variable replacement |
| Factory Method | `SesEmailSender::new()` | Create appropriate transport from config |

### Refactoring Rules
- **Extract Method** — if function > 30 lines, extract sub-functions
- **Replace Conditional with Polymorphism** — use trait dispatch over growing match chains
- **Introduce Parameter Object** — group related params into structs
- **Separate Query from Modifier** — read-only methods must not have side effects
```

### Step 2: Update Build Commands section (replace existing one)

Replace the existing Build Commands with the updated version that includes `cargo deny check` and `cargo llvm-cov`.

### Step 3: Verify CLAUDE.md is well-formed

Read the file back and ensure no formatting issues.

### Step 4: Commit

```bash
git commit -am "docs: update CLAUDE.md with guard rails, design patterns, refactoring rules"
```

---

## Task 11: Update .gitignore

### Files
- Modify: `.gitignore`

### Step 1: Add CI/coverage/release entries

Add to `.gitignore`:

```
# Coverage
coverage-html/
lcov.info
*.profraw

# Release
/deploy/
```

### Step 2: Commit

```bash
git commit -am "chore: update .gitignore for coverage and release artifacts"
```

---

## Task 12: Final Verification

### Step 1: Run full verification suite

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All must pass with zero warnings/errors.

### Step 2: Verify file structure

```bash
ls -la rust-toolchain.toml rustfmt.toml .clippy.toml deny.toml bacon.toml codecov.yml
ls -la .cargo/config.toml
ls -la .github/workflows/
ls -la release-please-config.json .release-please-manifest.json
```

Expected files:
```
.cargo/config.toml
.clippy.toml
.github/workflows/ci.yml
.github/workflows/claude.yml
.github/workflows/release-please.yml
.github/workflows/security.yml
.github/workflows/weekly-digest.yml
.release-please-manifest.json
bacon.toml
codecov.yml
deny.toml
release-please-config.json
rust-toolchain.toml
rustfmt.toml
```

### Step 3: Commit any remaining changes

```bash
git status
# If clean: done
# If changes: commit appropriately
```

---

## Verification Checklist

After all tasks complete:

```bash
# Full test suite
cargo test --workspace

# Lint
cargo clippy --workspace --all-targets -- -D warnings

# Format check
cargo fmt --all -- --check

# Verify all config files exist
ls rust-toolchain.toml rustfmt.toml .clippy.toml deny.toml bacon.toml codecov.yml
ls .cargo/config.toml
ls .github/workflows/*.yml
ls release-please-config.json .release-please-manifest.json
```

All must pass before PR.
