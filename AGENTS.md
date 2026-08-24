# AGENTS.md

Guidance for AI coding agents (opencode, Cursor, Claude Code, etc.) working on this
local clone of `cyborgoat/nest`. Read this before making changes.

## Remote configuration

| Remote | URL | Purpose |
|--------|-----|---------|
| `upstream` | `git@github.com:cyborgoat/nest.git` | Public main repo. **Fetch only, never push.** |
| `origin` | `git@github.com:stupidT/nest.git` | Personal fork. Push here for backup and PR source. |

SSH key: `~/.ssh/id_ed25519_github` (via `~/.ssh/config` Host `github.com`).

Fork default branch is set to `personal/dev` on GitHub to suppress unwanted
"Compare & pull request" banners for routine pushes.

## Branching strategy (authoritative)

This clone uses a **two-tier branch model** to keep upstream PRs clean while allowing
local experimentation, intermediate design docs, and agent-constraint notes.

```
upstream/main ──(fetch/reset)──>  main  ──(rebase)──>  personal/dev  ──(cut)──>  feat/xxx
                                                                                        │
                                    feat/xxx-clean  <──  cherry-pick final ────────────┘
                                          │
                                  push to origin (fork)
                                          │
                                       PR → upstream/dev
                                          │
                   (integration & cross-platform testing on dev)
                                          │
                             dev → main (by upstream maintainer)
```

Upstream maintains a `dev` integration branch. All PRs target `cyborgoat/nest:dev`;
the upstream maintainer merges `dev` into `main` once features are validated across
platforms.

### Branches

| Branch | Lifetime | Purpose | Pushed? |
|--------|----------|---------|---------|
| `main` | permanent | Mirrors `upstream/main` (production reference). **Never commit here.** Sync only via `git fetch upstream && git reset --hard upstream/main`. | fetch only, never push |
| `dev` | permanent | Mirrors `upstream/dev` (integration branch). **Never commit here.** Sync only via `git fetch upstream && git reset --hard upstream/dev`. | fetch only, never push |
| `personal/dev` | permanent | Working trunk for this developer. Holds agent constraints, intermediate design docs, experimental code, WIP commits. Rebase onto `dev` after each upstream sync. | push to `origin` (fork) for backup |
| `feat/xxx` | short-lived | Feature work cut from `personal/dev`. May contain messy history. | optional, local |
| `feat/xxx-clean` | short-lived | Clean branch for the upstream PR. Built by cherry-picking or interactive-rebasing only the **final** commits from `feat/xxx`. | push to `origin` (fork), PR target |

### Rules

1. **Never commit to `main`.** It mirrors upstream. Sync only.
2. **Never push to `upstream`.** It is the public repo. All pushes go to `origin` (fork).
3. **Never open a PR from `personal/dev`.** It contains intermediate state.
4. **PRs come from `feat/xxx-clean` branches only**, containing solely:
   - final production code
   - final user-facing docs (under `docs/` or root)
   - tests
5. **Intermediate artifacts stay on `personal/dev`** and never reach clean branches:
   - design drafts, exploration notes (put under `docs/_wip/`)
   - agent constraints, prompt experiments (this file and siblings)
   - scratch scripts, throwaway spikes
6. **Before opening a PR**, rebase `feat/xxx-clean` onto latest `upstream/dev`
   and run all sanity checks below.

### Daily workflow

```powershell
# --- Sync upstream changes ---
git checkout main
git fetch upstream
git reset --hard upstream/main

git checkout dev
git reset --hard upstream/dev

# --- Rebase personal work onto latest dev (integration state) ---
git checkout personal/dev
git rebase dev

# --- Push backup to fork ---
git push origin personal/dev
```

### Suggested flow for a new feature

```powershell
# from personal/dev, up to date with upstream/dev
git checkout -b feat/my-feature
# ... work, commit freely (messy history ok) ...

# when ready to PR:
git fetch upstream
git checkout -b feat/my-feature-clean upstream/dev
git cherry-pick <final-commit-sha-1> <final-commit-sha-2>
# run sanity checks (see below), then push to fork and open PR
git push origin feat/my-feature-clean
# open PR on GitHub: feat/my-feature-clean → cyborgoat/nest:dev
```

## Repository layout

```
apps/desktop      Tauri v2 + React (Rust core in src-tauri/)
apps/hub          NestJS Hub API (accounts, publishing, registry, messages)
apps/admin        React operations console, built into apps/hub/public/admin
packages/shared   Shared TypeScript types
examples/knowledge-packs   PyPI-style pack registry fixture
docs/             Architecture and development notes (canonical user-facing docs)
docs/_wip/        Local-only intermediate design docs (NOT for upstream PRs)
scripts/          Repo tooling (e.g. validate-pack-registry.mjs)
```

See `README.md` and `docs/development.md` for full prerequisites and run steps.

## Sanity checks (run before declaring a task done)

Agents MUST run the relevant checks below after non-trivial changes. If a command
fails, fix it before reporting completion. Do not skip with `--no-verify`.

```powershell
# Desktop UI (apps/desktop)
cd apps/desktop; npm run lint; npm test; npm run build

# Desktop Rust (apps/desktop/src-tauri)
cd apps/desktop/src-tauri; cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test

# Admin console (apps/admin)
cd apps/admin; npm run lint; npm test; npm run build

# Hub (apps/hub) — build also rebuilds admin into apps/hub/public/admin
cd apps/hub; npm run lint; npm test -- --runInBand; npm run build; npm run test:e2e -- --runInBand; npm run validate:registry
```

Only run the checks for the area you changed. If unsure which apply, run all of them.

## Working conventions

- **Comments**: do not add comments unless explicitly requested.
- **Style**: match existing file conventions; for Rust run `cargo fmt`, for TS/JS
  run the per-app `npm run lint` autofix.
- **Commits**: follow the existing Conventional Commits style seen in `git log`
  (e.g. `feat(desktop): ...`, `fix(hub): ...`, `refactor(desktop): ...`,
  `style(desktop): ...`, `docs: ...`). Keep subjects under ~72 chars.
- **Docs**: canonical user-facing docs live under `docs/` and the root `README.md`.
  Intermediate design notes go under `docs/_wip/` and must not be included in
  `feat/xxx-clean` PR branches.
- **Secrets**: never commit `.env`, API keys, or tokens. `.env.example` files are
  the only env files that belong in git.
- **Releases**: version bumps must update all three of
  `apps/desktop/src-tauri/tauri.conf.json`, `apps/desktop/package.json`, and
  `apps/desktop/src-tauri/Cargo.toml` to the same value. Do not cut tags unless
  explicitly asked.

## Local-only artifacts

The following are intentional local-only files on `personal/dev` and must not
appear in upstream PRs:

- `AGENTS.md` (this file)
- `docs/_wip/**` intermediate design docs
- any `*.scratch.*` or `*.local.*` files

When building `feat/xxx-clean`, exclude these via cherry-pick selection or by
resetting them off the clean branch before pushing.
