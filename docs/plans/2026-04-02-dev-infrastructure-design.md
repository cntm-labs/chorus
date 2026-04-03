# Chorus Development Infrastructure Design

> **Date:** 2026-04-02
> **Status:** Approved
> **Goal:** Set up production-grade development infrastructure for Chorus CPaaS — tooling, CI/CD, coverage, linting, pre-commit hooks, CLAUDE.md guard rails.

## 1. Tooling Config Files

| File | Purpose |
|------|---------|
| `rustfmt.toml` | Format: edition 2021, max_width 100, import grouping |
| `.clippy.toml` | Clippy config: MSRV, disallowed names |
| `deny.toml` | Dependency audit: licenses, advisories, bans |
| `rust-toolchain.toml` | Pin stable + components (rustfmt, clippy, llvm-tools-preview) |
| `.cargo/config.toml` | Cargo aliases (check-all, lint, test-all) |
| `bacon.toml` | Dev watch: check, clippy, test, nextest, fmt |
| `codecov.yml` | Coverage config: per-crate flags, thresholds |

### rustfmt.toml
```toml
edition = "2021"
max_width = 100
use_field_init_shorthand = true
group_imports = "StdExternalCrate"
imports_granularity = "Crate"
```

### .clippy.toml
```toml
msrv = "1.85.0"
disallowed-names = ["foo", "bar", "baz", "tmp", "temp"]
too-many-arguments-threshold = 10
```

### rust-toolchain.toml
```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy", "llvm-tools-preview"]
```

### Workspace lints (Cargo.toml additions)
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

### deny.toml
Adapted from nucleus — license whitelist (MIT, Apache-2.0, BSD, ISC, etc.), advisory DB checks, wildcard deny, unknown registry deny.

### .cargo/config.toml
```toml
[alias]
check-all = "check --workspace --all-targets"
test-all = "test --workspace"
lint = "clippy --workspace --all-targets -- -D warnings"
```

### bacon.toml
Watch-mode dev tool with keybindings: c=clippy, t=test, n=nextest, f=fmt.

### codecov.yml
Per-crate flags (core, providers), patch target 70%, project threshold 5%, ignore benches/sdks/load-tests.

## 2. GitHub CI Workflows

### ci.yml (push/PR to main)
- **fmt** — `cargo fmt --all -- --check`
- **clippy** — `cargo clippy --workspace --all-targets -- -D warnings`
- **test** — `cargo nextest run --workspace` + `cargo llvm-cov` → upload Codecov + HTML artifact
- **deny** — `cargo deny check`

### security.yml (push/PR)
- cargo-audit
- cargo-deny (EmbarkStudios/cargo-deny-action)

### claude.yml (PR/issue @claude)
- Claude Code Action for PR analysis and issue triage
- Concurrency group per issue/PR number
- Prompt tailored to CPaaS: security, API contract, test coverage

### weekly-digest.yml (cron Monday 9am)
- Claude generates weekly engineering digest

### release-please.yml (push to main)
- Automated changelog + version bumping
- CI gate (fmt + clippy + test) before publish
- Docker build + push to ghcr.io on tag

## 3. CLAUDE.md — AI Development Guard Rails

### Anti-patterns
- No `#[allow(dead_code)]` — use it or remove it
- No duplicate functions/structs — extract shared logic to chorus-core
- No spaghetti dependencies — follow dependency rules strictly
- No magic numbers — use constants
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
- Extract Method — if function > 30 lines, extract sub-functions
- Replace Conditional with Polymorphism — use trait dispatch over growing match chains
- Introduce Parameter Object — group related params into structs
- Separate Query from Modifier — read-only methods must not have side effects

### Pre-commit Hooks
- `cargo fmt --all -- --check`
- `cargo clippy --workspace -- -D warnings`

## 4. References

- Nucleus project (../nucleus) — CI, tooling, CLAUDE.md patterns
- NeuronOS kernel (../neuronos/neuron-kernel) — lint strictness, codecov, bacon
- refactoring.guru/design-patterns — pattern catalog
- refactoring.guru/refactoring/catalog — refactoring techniques
