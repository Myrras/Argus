# Argus — development tasks. Run `just` to see them.
# Shared recipes (dev, dev-signed, build, install-signed, check, test-core,
# format, version, version-check, rust, frontend, ci, build-signed, sweep) come
# from the rust-base clone: `ln -s /path/to/rust-base ~/.rust-base`.
# Don't redefine them here.
import? '~/.rust-base/ci/common.just'

# Show available recipes.
default:
    @just --list

# Needs rsvg-convert (`brew install librsvg`). Only the desktop icons are kept.

# Regenerate the app icons from src-tauri/icons/source/argus.svg.
icons:
    rsvg-convert -w 1024 -h 1024 src-tauri/icons/source/argus.svg -o src-tauri/icons/source/argus-1024.png
    pnpm tauri icon src-tauri/icons/source/argus-1024.png
    rm -rf src-tauri/icons/android src-tauri/icons/ios

# Needs cargo-llvm-cov (`rustup component add llvm-tools-preview &&
# cargo install cargo-llvm-cov`); HTML lands in target/llvm-cov/html, the
# frontend's (Vitest + v8) in coverage/.

# Line coverage of both suites (not part of `just ci`).
coverage:
    @command -v cargo-llvm-cov >/dev/null 2>&1 \
        || { echo "cargo-llvm-cov not installed: rustup component add llvm-tools-preview && cargo install cargo-llvm-cov"; exit 1; }
    cargo llvm-cov --workspace --no-report
    cargo llvm-cov report --summary-only --ignore-filename-regex '(testutil|guarantees)\.rs$'
    cargo llvm-cov report --html --ignore-filename-regex '(testutil|guarantees)\.rs$'
    pnpm test:coverage

# Only the integration test binary: `#[ignore]` also marks measurements such as
# `big_review_timings`, which must not run here. Run by the nightly workflow.

# Real GitHub API, read-only (needs gh and INTEGRATION_TOKEN; skips without it).
test-integration:
    cargo test -p argus-core --test github_integration -- --ignored --nocapture
