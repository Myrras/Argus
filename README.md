# Argus

A desktop review cockpit for GitLab merge requests and GitHub pull requests.
Rust + Tauri 2 + React + TypeScript + Vite. Forked from
[rust-application-boilerplate](https://github.com/RobbieMinderhoud/rust-application-boilerplate).

## Prerequisites

- Rust stable (`rust-toolchain.toml` pins `rustfmt` + `clippy`)
- Node 22 + `pnpm` 10
- Tauri 2 system prerequisites: <https://tauri.app/start/prerequisites/>
- `just` (`brew install just`)
- A clone of [`ci-workflows`](https://github.com/RobbieMinderhoud/ci-workflows), symlinked as `~/.ci-workflows` (for the shared recipes: `just dev`, `just ci`, `just build`, …)

## Get started

```
pnpm install
just dev    # run the app (hot reload)
just ci     # fmt, clippy, tests, eslint, vitest, build (shared common.just)
just build  # .app + .dmg
```

CI runs the shared workflows from `ci-workflows`; `just test-integration` runs
the opt-in tests against the real GitHub API (needs `gh` and `INTEGRATION_TOKEN`).

See `CLAUDE.md` for the layout and conventions, `CHANGELOG.md` for releases.
