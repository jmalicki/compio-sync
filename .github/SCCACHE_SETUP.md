# sccache Integration for GitHub Actions

This document describes the sccache integration and phased CI pipeline added to the project.

## Overview

[sccache](https://github.com/mozilla/sccache) is a compiler cache that speeds up Rust compilation by caching compilation artifacts. This is especially useful in CI environments where builds run frequently.

## CI Pipeline Phases

The CI workflow is now structured in **4 phases** that run sequentially, with each phase depending on the previous one:

### Phase 1: Code Quality (Runs First)
- **Job**: `code-quality`
- **Purpose**: Fast feedback on formatting and linting issues
- **Runs**: 
  - `cargo fmt` - formatting check
  - `cargo clippy` - linting with sccache
- **Benefits**: Fails fast if code doesn't meet quality standards, saving CI time

### Phase 2: Build
- **Jobs**: `build` (matrix: stable + nightly)
- **Purpose**: Compile all artifacts with sccache
- **Builds**:
  - Library code
  - Test binaries
  - Benchmark binaries (stable only)
- **sccache**: Caches compilation artifacts for use by test phase

### Phase 3: Test
- **Jobs**: `test` (matrix: stable + nightly)
- **Purpose**: Run tests using cached builds from Phase 2
- **Runs**:
  - Unit tests
  - Integration tests
  - Doc tests
- **Benefits**: Tests reuse sccache artifacts from build phase

### Phase 4: Documentation
- **Job**: `docs`
- **Purpose**: Build and verify documentation
- **Runs**: `cargo doc` with warning-as-error
- **Benefits**: Reuses sccache artifacts from previous phases

### Final Gate: CI Success
- **Job**: `ci-success`
- **Purpose**: Single status check that indicates all CI phases passed
- **Benefits**: Simplifies branch protection rules

## What Changed

### 1. Custom Composite Action (`.github/actions/setup-rust-sccache/action.yml`)

A reusable composite action that:
- Installs the Rust toolchain using `actions-rust-lang/setup-rust-toolchain@v1.15.2`
- Sets up sccache using `mozilla-actions/sccache-action@v0.0.7`
- Caches Rust dependencies with `Swatinem/rust-cache@v2`
- Caches sccache artifacts explicitly with `actions/cache@v4`

**Key improvements for cross-job caching:**
- Cache keys include OS and Rust toolchain version
- `save-if: "true"` enables caching from all branches
- Restore keys allow fallback to similar caches
- Shared keys enable cache reuse across jobs in the same workflow

### 2. Phased CI Workflow (`.github/workflows/ci.yml`)

Changes include:
- **Phased Structure**: 4 sequential phases with proper dependencies
- **Environment Variables**: Added `RUSTC_WRAPPER=sccache` and `SCCACHE_GHA_ENABLED=true` to enable sccache
- **Concurrency Control**: Added concurrency group to cancel in-progress runs when new commits are pushed
- **Matrix Strategy**: Build and test on stable + nightly (extensible to multiple OS)
- **Modernized Actions**: Updated from `actions/checkout@v3` to `v4`
- **Custom Action Usage**: Replaced `actions-rs/toolchain@v1` with the new composite action
- **Statistics Output**: Added sccache statistics output at the end of each job for monitoring

## Expected Benefits

1. **Faster Build Times**: sccache caches compilation artifacts, significantly speeding up builds
2. **Cost Savings**: Reduced CI minutes consumption
3. **Better Developer Experience**: Faster feedback on PRs
4. **Cache Hit Visibility**: Statistics output shows cache effectiveness

## Monitoring

After each CI run, check the "Show sccache statistics" step to see:
- Cache hit rate
- Number of compilations cached
- Cache size and usage

## Reference

This implementation is based on the sccache integration in the [jmalicki/arsync](https://github.com/jmalicki/arsync) repository.

## Technical Details

### Cache Strategy

The setup uses a two-layer caching approach:

1. **rust-cache**: Caches Cargo dependencies and build artifacts
   - Shared across runs with the same `Cargo.lock`
   - Only saved on main branch pushes to avoid cache pollution from PRs

2. **sccache artifacts**: Explicit caching of compiled object files
   - Persisted in `~/.cache/sccache`
   - Keyed by OS and `Cargo.lock` hash
   - Falls back to OS-specific cache if exact match not found

### Environment Variables

- `RUSTC_WRAPPER=sccache`: Tells Cargo to use sccache as the compiler wrapper
- `SCCACHE_GHA_ENABLED=true`: Enables GitHub Actions-specific optimizations in sccache
- `RUST_BACKTRACE=1`: Helpful for debugging build issues

### Concurrency Control

```yaml
concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true
```

This cancels old CI runs when new commits are pushed to the same PR/branch, saving CI minutes.

