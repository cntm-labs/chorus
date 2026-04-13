# chorus-core → `use chorus::` SDK Rename

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Change `chorus-core`'s library name so developers write `use chorus::client::Chorus` instead of `use chorus_core::client::Chorus`.

**Architecture:** Add `[lib] name = "chorus"` to `chorus-core/Cargo.toml`. This changes the Rust import name from `chorus_core` to `chorus` while keeping the crates.io package name as `chorus-core`. All internal crates (`chorus-providers`, `chorus-server`) must update their imports. No new crates are created — this is a rename, not a restructure.

**Tech Stack:** Rust, Cargo workspace

**Note on providers:** `chorus-core` cannot re-export `chorus-providers` (circular dependency). Provider imports remain `use chorus_providers::sms::TwilioSmsSender`. This is acceptable — provider imports are rare (1-2 lines in setup code), while core imports (`types`, `client`, `templates`) appear everywhere.

---

## Task 1: Add `[lib]` section to chorus-core

### Files
- Modify: `crates/chorus-core/Cargo.toml`

### Step 1: Add lib name

Add after the `[package]` section, before `[dependencies]`:

```toml
[lib]
name = "chorus"
```

### Step 2: Verify compilation fails

Run: `cargo check --workspace`
Expected: FAIL — all `use chorus_core::` imports now broken (lib name changed to `chorus`)

### Step 3: Commit

```bash
git add crates/chorus-core/Cargo.toml
git commit -m "feat(core): set lib name to 'chorus' for cleaner imports"
```

---

## Task 2: Update chorus-core internal imports

### Files
- Modify: `crates/chorus-core/src/lib.rs` (doc example)

### Step 1: Update doc example

Replace `use chorus_core::` with `use chorus::` in the doc comment example:

```rust
//! ```rust,no_run
//! use chorus::client::Chorus;
//! use chorus::types::SmsMessage;
//!
//! # async fn example() -> Result<(), chorus::error::ChorusError> {
```

### Step 2: Verify chorus-core compiles

Run: `cargo check -p chorus-core`
Expected: PASS

### Step 3: Commit

```bash
git add crates/chorus-core/src/lib.rs
git commit -m "docs(core): update doc examples to use chorus:: imports"
```

---

## Task 3: Update chorus-providers imports

### Files
- Modify: `crates/chorus-providers/src/sms/twilio.rs`
- Modify: `crates/chorus-providers/src/sms/telnyx.rs`
- Modify: `crates/chorus-providers/src/sms/plivo.rs`
- Modify: `crates/chorus-providers/src/sms/mock.rs`
- Modify: `crates/chorus-providers/src/email/mock.rs`
- Modify: `crates/chorus-providers/src/email/resend.rs`
- Modify: `crates/chorus-providers/src/email/ses.rs`
- Modify: `crates/chorus-providers/src/email/smtp.rs`

### Step 1: Replace all `chorus_core::` with `chorus::` in all 8 files

In every file, replace:
```rust
use chorus_core::error::ChorusError;
use chorus_core::sms::SmsSender;
// etc.
```
With:
```rust
use chorus::error::ChorusError;
use chorus::sms::SmsSender;
// etc.
```

### Step 2: Verify compilation

Run: `cargo check -p chorus-providers`
Expected: PASS

### Step 3: Run tests

Run: `cargo test -p chorus-providers`
Expected: All tests pass

### Step 4: Commit

```bash
git add crates/chorus-providers/src/
git commit -m "refactor(providers): update imports from chorus_core to chorus"
```

---

## Task 4: Update chorus-server imports

### Files
- Modify: `crates/chorus-server/src/queue/router_builder.rs`
- Modify: `crates/chorus-server/src/queue/worker.rs`
- Modify: any other files with `use chorus_core::`

### Step 1: Find and replace all `chorus_core::` with `chorus::`

Search all files in `crates/chorus-server/src/` for `chorus_core::` and replace with `chorus::`.

### Step 2: Verify compilation

Run: `cargo check -p chorus-server`
Expected: PASS

### Step 3: Run full test suite

Run: `cargo test --workspace`
Expected: All tests pass

### Step 4: Commit

```bash
git add crates/chorus-server/src/
git commit -m "refactor(server): update imports from chorus_core to chorus"
```

---

## Task 5: Update documentation and integration guide

### Files
- Modify: `docs/guides/auth-service-integration.md`
- Modify: `CLAUDE.md` (if references `chorus_core::`)
- Modify: `README.md` (if exists and references `chorus_core::`)

### Step 1: Replace `chorus_core::` with `chorus::` in all docs

### Step 2: Commit

```bash
git add docs/ CLAUDE.md README.md
git commit -m "docs: update all references from chorus_core to chorus"
```

---

## Task 6: Final verification

### Step 1: Run full CI checks

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo deny check
```

### Step 2: Verify the import works as expected

Create a quick test in chorus-core to confirm:

```rust
#[test]
fn sdk_import_name() {
    // Verify the crate is importable as `chorus::`
    // This test existing and compiling proves the lib name works
    let _ = chorus::client::Chorus::builder();
}
```

### Step 3: Commit

```bash
git add crates/chorus-core/
git commit -m "test(core): verify chorus:: import name works"
```

---

## Verification Checklist

After all tasks complete:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo deny check
```

All must pass before merging.

## Result

Before:
```rust
use chorus_core::client::Chorus;
use chorus_core::types::SmsMessage;
```

After:
```rust
use chorus::client::Chorus;
use chorus::types::SmsMessage;
```

crates.io name remains `chorus-core` (`cargo add chorus-core`), but Rust import is now `chorus::`.
