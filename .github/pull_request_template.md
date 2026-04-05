## Summary

<!-- Brief description of the changes -->

## Changes

-

## Test Plan

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --workspace -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] New tests added for new functionality

## Checklist

- [ ] Code follows project conventions (see CLAUDE.md)
- [ ] No `dbg!()`, `print!()`, `todo!()` in production code
- [ ] Public APIs are documented
- [ ] Dependency rules respected (core → no internal deps, providers → core only)
