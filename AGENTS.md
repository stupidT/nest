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
upstream/main ──(fetch/reset)──>  main  ──(release PR)──>  upstream/dev ──(fetch/reset)──>  dev
                                                                                            │
                                                               personal/dev <──(rebase)──┘
                                                                        │
                                                                (cut)──>  feat/xxx
                                                                        │
                                  milestone backflow (WIP code + local artifacts) ─────┘
                                                                                        │
                                                           cherry-pick final commits │
                                                                                        ▼
                                                                                feat/xxx-clean
                                                                                        │
                                                                              push to origin (fork)
                                                                                        │
                                                                                  PR → upstream/dev
```

Upstream maintains a `dev` integration branch. All PRs target `cyborgoat/nest:dev`;
the upstream maintainer merges `dev` into `main` via release PRs once features are
validated across platforms.

### Branches

| Branch | Lifetime | Purpose | Pushed? |
|--------|----------|---------|---------|
| `main` | permanent | Mirrors `upstream/main` (production reference). **Never commit here.** Sync only via `git fetch upstream && git reset --hard upstream/main`. | fetch only, never push |
| `dev` | permanent | Mirrors `upstream/dev` (integration branch). **Never commit here.** Sync only via `git fetch upstream && git reset --hard upstream/dev`. | fetch only, never push |
| `personal/dev` | permanent | Working trunk for this developer. Holds agent constraints, intermediate design docs, and **all current WIP** — feature branches flow their work and local artifacts back here at every milestone, so the trunk never falls behind an active feature branch. Rebase onto `dev` after each upstream sync. | push to `origin` (fork); `origin` is the canonical source of truth |
| `feat/xxx` | short-lived | Feature work cut from `personal/dev`. May contain messy history. | push to `origin` (fork); `origin` is the canonical remote for cross-machine work |
| `feat/xxx-clean` | short-lived, **single-use** | Clean branch for exactly one upstream PR. Built by cherry-picking or interactive-rebasing only the **final** commits from `feat/xxx`. **Deleted immediately after its PR merges; never add commits after the PR merges** — follow-up fixes get a new clean branch. | push to `origin` (fork), PR source |
| `archive/<name>` | safety pointer only | Optional one-time pointer created before retiring a long-lived branch. Delete once its content is confirmed preserved on `personal/dev` or upstream. | push to `origin` (fork) only while unsure |

### Rules

1. **Never commit to `main` or `dev`.** They mirror upstream. Sync only.
2. **Never push to `upstream`.** It is the public repo. All pushes go to `origin` (fork).
3. **Never open a PR from `personal/dev`.** It contains intermediate state.
4. **PRs come from `feat/xxx-clean` branches only, and target `upstream/dev`** (not
   `upstream/main`; upstream integrates `dev` → `main` via maintainer release PRs).
   A clean branch contains solely:
   - final production code
   - final user-facing docs (under `docs/` or root)
   - tests
5. **Intermediate artifacts stay on `personal/dev`** and never reach clean branches:
   - design drafts, exploration notes (put under `docs/_wip/`)
   - agent constraints, prompt experiments (this file and siblings)
   - scratch scripts, throwaway spikes
6. **Distill at every milestone.** When a D-slice, milestone, or other reviewable unit
   of work completes on `feat/xxx`, immediately distill its final commits into a clean
   branch and open the PR. Never let a feature branch accumulate more than one
   milestone of undistilled work.
7. **Backflow keeps the trunk current.** After every distill (and every PR merge),
   merge or rebase `feat/xxx` back into `personal/dev` so the trunk holds the newest
   WIP code plus local artifacts. `personal/dev` must never be emptier than an active
   feature branch.
8. **Post-merge reconciliation, then retirement.** After a PR merges:
   - sync `main` and `dev`, rebase `personal/dev`;
   - tree-diff the feature branch against the merged clean tip:
     `git diff <clean-tip> feat/xxx --stat` — anything beyond the local artifacts
     listed below is real unmerged work: distill it into a new clean branch and
     restore artifacts onto `personal/dev`;
   - delete `feat/xxx` and `feat/xxx-clean` (local and `origin`).
   Commit counts and `git cherry` are unreliable after squash merges; trust tree diffs.
9. **`origin` is the source of truth across machines.** Push `personal/dev` and active
   `feat/*` branches at the end of every session; `git pull --rebase` at the start.
   Never leave work local-only.
10. **Before opening a PR**, base `feat/xxx-clean` on latest `upstream/dev` and run
    all sanity checks below.

These rules exist because of real failures found in the August 2026 branch cleanup:
the trunk starved to "main + docs" while all WIP lived on one feature branch; a merged
clean branch was reused and stranded an orphan commit; an entire epic accumulated
undistilled because distillation had no milestone rhythm; and squash-merge made commit
counts and `git cherry` useless for working out what was still unmerged.

### Daily workflow

```powershell
# --- Session start: sync from fork, then upstream ---
git fetch origin --prune
git checkout personal/dev
git pull --rebase origin personal/dev

# --- Sync upstream changes ---
git checkout main
git fetch upstream
git reset --hard upstream/main

git checkout dev
git reset --hard upstream/dev

# --- Rebase personal work onto latest dev (integration state) ---
git checkout personal/dev
git rebase dev

# --- Session end: push everything to the fork ---
git push origin personal/dev
git push origin feat/<active-branch>
```

### Suggested flow for a new feature

```powershell
# from personal/dev, up to date with dev
git checkout -b feat/my-feature
git push -u origin feat/my-feature
# ... work, commit freely (messy history ok), push at session end ...

# at each milestone — distill immediately, do not accumulate:
git fetch upstream
git checkout -b feat/my-feature-m1-clean upstream/dev
git cherry-pick <final-commit-sha-1> <final-commit-sha-2>
# run sanity checks (see below), then push to fork and open PR
git push origin feat/my-feature-m1-clean
# open PR on GitHub: feat/my-feature-m1-clean → cyborgoat/nest:dev

# after the PR merges — reconcile and retire:
git checkout main
git fetch upstream
git reset --hard upstream/main
git checkout dev
git reset --hard upstream/dev
git checkout personal/dev
git rebase dev
git diff <clean-tip> feat/my-feature --stat   # expect only local artifacts
git branch -D feat/my-feature feat/my-feature-m1-clean
git push origin --delete feat/my-feature feat/my-feature-m1-clean
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
- `CONTEXT-MAP.md`, `apps/desktop/CONTEXT.md`, `docs/agents/**`
- `docs/_wip/**` intermediate design docs
- `apps/desktop/tauri-dev.cmd` and other local dev scripts
- local-only `.gitignore` entries (`.opencode/`, `graphify-out/`, `.codex/`)
- `packages/shared/package-lock.json` local install drift
- any `*.scratch.*` or `*.local.*` files

When building `feat/xxx-clean`, exclude these via cherry-pick selection or by
resetting them off the clean branch before pushing. In post-merge reconciliation
(rule 8), these files are the expected residue of `git diff <clean-tip> feat/xxx`.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).

## Agent skills

### Issue tracker

Issues and specs are tracked as local Markdown files under `.scratch/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Uses the default five-role triage vocabulary. See `docs/agents/triage-labels.md`.

### Domain docs

Uses a multi-context domain-document layout. See `docs/agents/domain.md`.
