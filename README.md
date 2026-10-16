# Argus

A desktop review cockpit for GitLab merge requests and GitHub pull requests.
Rust + Tauri 2 + React + TypeScript + Vite. Forked from
[rust-application-boilerplate](https://github.com/RobbieMinderhoud/rust-application-boilerplate).

## Prerequisites

- Rust stable (`rust-toolchain.toml` pins `rustfmt` + `clippy`)
- Node 22 + `pnpm` 10
- Tauri 2 system prerequisites: <https://tauri.app/start/prerequisites/>
- `just` (`brew install just`)

## Get started

```
pnpm install
just dev    # run the app (hot reload)
just test   # Rust + frontend tests
just lint   # clippy -D warnings + eslint
just build  # .app + .dmg
```

See `CLAUDE.md` for the layout and conventions, `CHANGELOG.md` for releases.
