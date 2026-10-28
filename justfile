# Argus — development tasks. Run `just` to see them.
# Shared recipes (rust, frontend, ci, build-signed, sweep) come from the
# ci-workflows clone: `ln -s /path/to/ci-workflows ~/.ci-workflows`.
import? '~/.ci-workflows/common.just'

# Show available recipes.
default:
    @just --list

# Run the app in development (Vite dev server + Tauri, hot reload).
dev:
    pnpm tauri dev

# Tauri's own dmg step uses HFS+, which is broken on macOS 26+, so we build
# the .app here and package the .dmg ourselves (APFS) via the shared make-dmg.sh.

# Build the production desktop bundle (.app + .dmg).
build:
    pnpm install
    pnpm tauri build --bundles app
    ~/.ci-workflows/scripts/make-dmg.sh
    @just sweep

# Build + launch a debug .app signed with a STABLE self-signed cert (macOS).
# `just dev` ad-hoc-signs the binary with a fresh identity every rebuild, so the
# keychain (GitLab token) re-prompts each run and "Always Allow" never sticks.
# This recipe keeps one signing identity across rebuilds, so a single "Always
# Allow" persists. Tradeoff: runs a bundle, so there's NO hot reload — use
# `just dev` for fast iteration and this when the keychain prompt is in your way.
# One-time setup: create a self-signed *Code Signing* certificate named
# "Argus Dev" (Keychain Access → Certificate Assistant → Create a Certificate);
# to use a different name, edit src-tauri/tauri.dev-signed.conf.json.
dev-signed: _signed-conf
    #!/usr/bin/env bash
    set -euo pipefail
    # Assigned apart from the export, so a failed lookup stops the recipe
    # (`export X="$(…)"` takes export's status and hides it from set -e).
    target_dir="$(just _target-dir)"
    export CARGO_TARGET_DIR="$target_dir"
    pnpm tauri build --debug --bundles app --config src-tauri/tauri.dev-signed.conf.json
    just sweep
    open "$CARGO_TARGET_DIR/debug/bundle/macos/Argus.app"

# Build the signed release and put it in /Applications (no Finder window).
# Copied next to it first and swapped in with mv, so a failed copy never
# leaves /Applications without the app.
install-signed: _build-signed
    #!/usr/bin/env bash
    set -euo pipefail
    app="$(just _target-dir)/release/bundle/macos/Argus.app"
    dest="/Applications/Argus.app"
    tmp="/Applications/.Argus.app.tmp"
    old="/Applications/.Argus.app.old"
    rm -rf "$tmp" "$old"
    ditto "$app" "$tmp"
    if [ -e "$dest" ]; then mv "$dest" "$old"; fi
    if ! mv "$tmp" "$dest"; then
        if [ -e "$old" ]; then mv "$old" "$dest"; fi
        echo "error: could not install $dest" >&2
        exit 1
    fi
    rm -rf "$old"
    echo "Installed → $dest"

# Regenerate the app icons from src-tauri/icons/source/argus.svg (needs
# rsvg-convert: `brew install librsvg`). Only the desktop icons are kept.
icons:
    rsvg-convert -w 1024 -h 1024 src-tauri/icons/source/argus.svg -o src-tauri/icons/source/argus-1024.png
    pnpm tauri icon src-tauri/icons/source/argus-1024.png
    rm -rf src-tauri/icons/android src-tauri/icons/ios

# Type-check the whole Rust workspace without building.
check:
    cargo check --workspace

# Rust needs cargo-llvm-cov (`rustup component add llvm-tools-preview &&
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

# Run only the core unit + integration tests.
test-core:
    cargo test -p argus-core

# Only the integration test binary: `#[ignore]` also marks measurements such as
# `big_review_timings`, which must not run here. Run by the nightly workflow.

# Real GitHub API, read-only (needs gh and INTEGRATION_TOKEN; skips without it).
test-integration:
    cargo test -p argus-core --test github_integration -- --ignored --nocapture

# Format Rust + frontend.
format:
    cargo fmt --all
    pnpm format

# Set the version and sync it across Cargo.toml, tauri.conf.json, package.json.
# Usage: just version 0.2.0
version new:
    bash ~/.ci-workflows/scripts/sync-version.sh --set {{ new }}

# Verify the version is identical across all three manifests.
version-check:
    bash ~/.ci-workflows/scripts/sync-version.sh --check
