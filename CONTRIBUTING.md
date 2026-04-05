# Contributing to Chorus

## Getting Started

1. Fork the repository
2. Create a feature branch: `git checkout -b feat/my-feature`
3. Make your changes
4. Run checks:
   ```sh
   cargo fmt --all -- --check
   cargo clippy --workspace -- -D warnings
   cargo test --workspace
   ```
5. Commit and open a pull request

## Conventions

See [CLAUDE.md](CLAUDE.md) for project conventions, lint policy, and architecture.

Key rules:
- All errors use `ChorusError` enum
- No `dbg!()`, `print!()`, `todo!()` in production code
- E.164 format required for phone numbers
- Template variables use `{{variable}}` syntax
- All public types/functions must have doc comments
- Max file ~300 lines — split if exceeding

## Pre-commit Hook

Enable the pre-commit hook to run checks automatically:

```sh
git config core.hooksPath .githooks
```

## Project Structure

```
crates/
├── chorus-core        # Traits, routing engine, types, errors (leaf crate)
├── chorus-providers   # Telnyx, Twilio, Plivo, Resend, SES, SMTP, Mock adapters
└── chorus-server      # Axum REST API, billing, dashboard
```

### Dependency Rules

- `chorus-core` → external deps only (leaf crate)
- `chorus-providers` → `chorus-core` + reqwest
- `chorus-server` → all crates (composition root)
