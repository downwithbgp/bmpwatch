# Release Process

Checklist for cutting a new bmpwatch release.

## Before tagging

```sh
# Verify clean working tree
git status

# Full workspace checks
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Verify crate packaging and release plan
cargo package --list -p bmpwatch
cargo publish --dry-run -p bmpwatch
cargo dist plan
```

## Tag and build

```sh
# Tag the release (triggers cargo-dist CI)
git tag v0.1.0
git push origin v0.1.0
```

The cargo-dist release workflow will:
- Build platform binaries for Linux (x86_64, ARM64) and macOS (x86_64, Apple Silicon)
- Create a draft GitHub Release with `.tar.xz` archives and SHA256 checksums

## Publish the GitHub Release

1. Wait for CI to complete
2. Review the draft release on GitHub
3. Verify archive contents: `bmpwatch` binary, LICENSE, README.md
4. Publish the release

## Publish to crates.io

After the GitHub Release is published:

```sh
cargo publish -p bmpwatch
```

This publishes only the main `bmpwatch` crate. The `record_openbmp_kafka`
companion tool is excluded from crates.io (`publish = false`).
