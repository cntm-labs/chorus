# CI crates.io Publish Design

**Goal:** Automate crates.io publishing via release-please so every release includes CHANGELOG updates, version bumps, and crate publishing.

## Current State

- `release-please.yml` creates release PRs with CHANGELOG, runs ci-gate, pushes Docker image
- `ci.yml` runs fmt, clippy, test, coverage on PRs
- Both crates (`chorus-core`, `chorus-providers`) published manually as `0.1.0-beta`
- `0.1.0` was yanked (published without docs)
- `CARGO_REGISTRY_TOKEN` secret already configured

## Design

### release-please config changes

Convert from single-package to multi-package tracking:

- `release-please-config.json`: define two packages (`crates/chorus-core`, `crates/chorus-providers`)
- `.release-please-manifest.json`: track versions per crate path

### publish-crates job

Add to `release-please.yml` after `ci-gate`:

```
ci-gate passes
  → publish chorus-core (must go first — it's a dependency)
  → wait for crates.io index propagation
  → publish chorus-providers
  → docker push (unchanged)
```

Key details:
- Uses `CARGO_REGISTRY_TOKEN` secret
- Publishes core first, then providers (dependency order)
- `--no-verify` flag since ci-gate already verified
- Sleep between publishes for crates.io index propagation
- Only runs when `releases_created == 'true'`

### Version flow

```
0.1.0-beta (current) → 0.1.1 (next patch) → 0.2.0 (next minor) → 1.0.0
```

`0.1.0` is permanently reserved (yanked). release-please will bump from `0.1.0-beta` onward.

## Out of Scope

- Per-crate independent versioning (both crates share one version for now)
- Separate CHANGELOG per crate
