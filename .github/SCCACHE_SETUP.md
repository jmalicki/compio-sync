# sccache Integration for GitHub Actions

This document describes the sccache integration added to the CI pipeline.

## Overview

[sccache](https://github.com/mozilla/sccache) is a compiler cache that speeds up Rust compilation by caching compilation artifacts. This is especially useful in CI environments where builds run frequently.

## What Changed

### 1. Custom Composite Action (`.github/actions/setup-rust-sccache/action.yml`)

A reusable composite action that:
- Installs the Rust toolchain using `actions-rust-lang/setup-rust-toolchain@v1.15.2`
- Sets up sccache using `mozilla-actions/sccache-action@v0.0.7`
- Caches Rust dependencies with `Swatinem/rust-cache@v2`
- Caches sccache artifacts explicitly with `actions/cache@v4`

### 2. Updated CI Workflow (`.github/workflows/ci.yml`)

Changes include:
- **Environment Variables**: Added `RUSTC_WRAPPER=sccache` and `SCCACHE_GHA_ENABLED=true` to enable sccache
- **Concurrency Control**: Added concurrency group to cancel in-progress runs when new commits are pushed
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

