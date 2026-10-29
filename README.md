# Argus

A desktop review cockpit for GitLab merge requests and GitHub pull requests.
Rust + Tauri 2 + React + TypeScript + Vite. Built on the shared
skeleton [`rust-base`](https://github.com/Myrras/rust-base).

## Prerequisites

- Rust stable (`rust-toolchain.toml` pins `rustfmt` + `clippy`)
- Node 22 + `pnpm` 10
- Tauri 2 system prerequisites: <https://tauri.app/start/prerequisites/>
- `just` (`brew install just`)
- A clone of [`rust-base`](https://github.com/Myrras/rust-base), symlinked as `~/.rust-base` (for the shared recipes: `just dev`, `just ci`, `just build`, …)

## Get started

```
pnpm install
just dev    # run the app (hot reload)
just ci     # fmt, clippy, tests, eslint, vitest, build (shared common.just)
just build  # .app + .dmg
```

CI runs the shared workflows from `rust-base` (settings in `.github/rust-base.json`); `just test-integration` runs
the opt-in tests against the real GitHub API (needs `gh` and `INTEGRATION_TOKEN`).

See `CLAUDE.md` for the layout and conventions, `CHANGELOG.md` for releases.
