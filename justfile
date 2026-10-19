# Argus — development tasks. Run `just` to see them.

# Show available recipes.
default:
    @just --list

# Run the app in development (Vite dev server + Tauri, hot reload).
dev:
    pnpm tauri dev

# Build the production desktop bundle (.app + .dmg).
# Tauri's own dmg step uses HFS+, which is broken on macOS 26+, so we build
# the .app here and package the .dmg ourselves (APFS) via scripts/make-dmg.sh.
build:
    pnpm install
    pnpm tauri build --bundles app
    ./scripts/make-dmg.sh
    @just sweep

# Reclaim stale build artifacts (old dependency versions + other toolchains)
# WITHOUT touching the current build, so `target/` stops growing without bound.
# Runs automatically at the tail of `build`. No-op with a hint if cargo-sweep
# isn't installed (`cargo install cargo-sweep`).
sweep:
    @command -v cargo-sweep >/dev/null 2>&1 \
        && cargo sweep --installed >/dev/null && cargo sweep --time 7 >/dev/null \
        && echo "recycled stale target/ artifacts" \
        || echo "cargo-sweep not installed; skipping target/ recycle (cargo install cargo-sweep)"

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
    pnpm tauri build --debug --bundles app --config src-tauri/tauri.dev-signed.conf.json
    @just sweep
    open "${CARGO_TARGET_DIR:-target}/debug/bundle/macos/Argus.app"

# Build a SIGNED release .app for personal install (macOS).
# Reuses the stable signing identity in tauri.dev-signed.conf.json (that file
# only sets bundle.macOS.signingIdentity, so it's equally valid for release).
# A stable identity means macOS keychain "Always Allow" persists and the app
# launches with no Gatekeeper prompt on this machine. Output:
# target/release/bundle/macos/Argus.app. Install: drag it into /Applications,
# or run `just install-signed`.
# Only the .app is built (--bundles app): the .dmg step isn't needed for a
# personal install.
build-signed: _signed-conf
    pnpm install
    pnpm tauri build --bundles app --config src-tauri/tauri.dev-signed.conf.json
    @just sweep
    open "${CARGO_TARGET_DIR:-target}/release/bundle/macos"

# Build the signed release and copy it straight into /Applications.
install-signed: build-signed
    rm -rf "/Applications/Argus.app"
    cp -R "${CARGO_TARGET_DIR:-target}/release/bundle/macos/Argus.app" "/Applications/Argus.app"
    @echo "Installed → /Applications/Argus.app"

# Guard for the signed recipes. The config is gitignored (it names *your*
# machine's signing identity), so a fresh clone or git worktree does not have it
# and tauri would fail with an opaque config error instead.
_signed-conf:
    @test -f src-tauri/tauri.dev-signed.conf.json || { echo "error: src-tauri/tauri.dev-signed.conf.json is missing."; echo "It is gitignored (machine-specific signing identity), so copy it from another checkout of this repo, or create it per 'macOS keychain prompt during development' in CLAUDE.md."; echo "Your identities: security find-identity -v -p codesigning"; exit 1; }

# Regenerate the app icons from src-tauri/icons/source/argus.svg (needs
# rsvg-convert: `brew install librsvg`). Only the desktop icons are kept.
icons:
    rsvg-convert -w 1024 -h 1024 src-tauri/icons/source/argus.svg -o src-tauri/icons/source/argus-1024.png
    pnpm tauri icon src-tauri/icons/source/argus-1024.png
    rm -rf src-tauri/icons/android src-tauri/icons/ios

# Type-check the whole Rust workspace without building.
check:
    cargo check --workspace

# Run all tests (Rust workspace + frontend).
test:
    cargo test --workspace
    -pnpm test

# Rust needs cargo-llvm-cov (`rustup component add llvm-tools-preview &&
# cargo install cargo-llvm-cov`); HTML lands in target/llvm-cov/html, the
# frontend's (Vitest + v8) in coverage/.
# Line coverage of both suites (not part of `just test` or CI).
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

# Lint: clippy with warnings-as-errors + frontend lint.
lint:
    cargo clippy --workspace --all-targets -- -D warnings
    -pnpm lint

# Format Rust + frontend.
format:
    cargo fmt --all
    -pnpm format

# Set the version and sync it across Cargo.toml, tauri.conf.json, package.json.
# Usage: just version 0.2.0
version new:
    bash ./scripts/sync-version.sh --set {{new}}

# Verify the version is identical across all three manifests.
version-check:
    bash ./scripts/sync-version.sh --check
