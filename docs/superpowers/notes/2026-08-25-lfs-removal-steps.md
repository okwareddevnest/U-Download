# Git LFS removal — exact steps for the user to run

Context: the repo's Git LFS budget is exhausted, which breaks `git lfs pull`
(binaries checked out as 3-line pointer stubs) and breaks CI release builds
(`.github/workflows/release.yml` used `lfs: true`). This task moved to
fetching `yt-dlp`, `ffmpeg`, and `aria2c` at build time via
`scripts/fetch-binaries.sh` instead of storing them in Git/Git LFS.

Per the task's hard constraint, this agent did **not** run any git write
commands (no `add`, `commit`, `rm`, `push`, `lfs`, etc). Everything below is
for you to run yourself, in order.

## What already changed in the working tree (not yet committed)

- `scripts/fetch-binaries.sh` — new script (downloads the three binaries).
- `.github/workflows/release.yml` — removed both `lfs: true` checkout options;
  added a "Fetch bundled binaries" step per build-matrix entry.
- `.gitattributes` — **deleted**. It contained only the three LFS tracking
  rules for `ffmpeg`, `yt-dlp`, `aria2c`; with those removed the file was
  empty, so it was removed entirely.
- `.gitignore` — added `src-tauri/binaries/` (under a new comment block,
  right after the existing `# Tauri` section). The pre-existing `*.md` /
  `!README.md` / `!docs/**/*.md` negation block was left untouched.
- `README.md` — added a short "Building from Source" section pointing at
  `scripts/fetch-binaries.sh`. The "Zero Dependencies" claims were left as-is
  — they're still true for people who download a built release; only the
  *build-time* binary source changed.
- `src-tauri/binaries/macos-arm64/{yt-dlp,ffmpeg,aria2c}` — **overwritten**
  by actually running the fetch script on this machine as part of
  verification (see task report for full transcript). These now contain
  real, working binaries instead of LFS pointer stubs. This was intentional
  and expected — see "Important note" below.

## ⚠️ Important gotcha: `.gitignore` will hide the new script

This repo's `.gitignore` has pre-existing blanket rules:

```
*.sh
scripts/*
```

These rules predate this change and were **not** modified (out of scope for
this task). Because of them, `scripts/fetch-binaries.sh` is a new/untracked
file that `git status` and `git add scripts/` will **not** pick up — it's
currently ignored, same as any other new `.sh` file would be. (The three
pre-existing scripts in `scripts/` show up in `git ls-files` only because
they were force-added before this ignore rule existed or applied to them.)

**You must force-add it explicitly:**

```bash
git add -f scripts/fetch-binaries.sh
```

If you'd rather not rely on `-f` going forward, consider adding an
exception (e.g. `!scripts/fetch-binaries.sh` or `!scripts/*.sh`) to
`.gitignore` — that edit was left for you since it touches a rule this task
wasn't asked to change.

## Step 1 — Stage the ordinary file changes

```bash
git add .gitignore README.md .github/workflows/release.yml
git add -f scripts/fetch-binaries.sh
```

`.gitattributes` was deleted by this agent (via `rm`, not `git rm`), so it
currently shows as a working-tree deletion. Stage that deletion too:

```bash
git add .gitattributes
```

(`git add` correctly stages a deletion for a file that no longer exists on
disk — no need for `git rm` here, though `git rm .gitattributes` would be
equivalent if you prefer to be explicit.)

## Step 2 — Untrack the binaries directory

This is the step that actually stops the repo from using Git LFS for these
files going forward:

```bash
git rm --cached -r src-tauri/binaries/
```

Notes on this command:

- `--cached` removes the files **from Git's index only** — it does **not**
  touch your working-tree copies. Your freshly-fetched, working
  `macos-arm64` binaries (and the still-stub-format files under the other
  four platform directories) stay on disk exactly as they are.
- After this, `src-tauri/binaries/` is untracked and covered by the
  `.gitignore` rule added above, so `git status` will show it as ignored,
  not as untracked clutter.
- This removes the *files at HEAD going forward* from every future commit
  and clone. It does **not** rewrite history — the LFS pointer files (and
  the LFS objects they pointed to) still exist in every prior commit that
  touched `src-tauri/binaries/`.

## Step 3 — Commit

```bash
git commit -m "build: fetch bundled binaries at build time instead of storing them in Git LFS"
```

(Adjust wording as you like — this agent does not create commits per the
task's hard constraint.)

## What this DOES and DOES NOT fix

**Fixes:**
- Future `git clone` / `git pull` no longer touch Git LFS for these files at
  all — new clones simply won't have `src-tauri/binaries/` populated, and
  CI/devs run `scripts/fetch-binaries.sh` instead.
- CI release builds are unblocked: `release.yml` no longer requests LFS
  objects (`lfs: true` removed from both checkout steps) and instead fetches
  fresh binaries per matrix target before the Tauri build step.
- No new LFS bandwidth/storage is consumed by this repo from this commit
  forward.

**Does NOT fix:**
- **The already-exhausted LFS storage/bandwidth is not reclaimed.** The old
  LFS objects (~380 MB worth, per the task background) remain referenced by
  every historical commit that touched `src-tauri/binaries/` before this
  change. `git rm --cached` only stops *new* commits from referencing them —
  it is not a history rewrite and does not delete anything from GitHub's LFS
  storage for this repo.
- If your GitHub LFS quota/budget is still reported as exceeded after this
  commit, that's expected. To actually reclaim quota you would need one of:
  - Rewriting history to strip `src-tauri/binaries/**` from every commit
    (e.g. `git filter-repo --path src-tauri/binaries --invert-paths`) and
    force-pushing — this rewrites all commit hashes on the branch(es)
    touched and requires coordinating with anyone else who has clones.
  - Contacting GitHub Support to ask about purging orphaned/unreferenced LFS
    objects or increasing the LFS data pack quota.
  - Simply waiting out the monthly bandwidth reset (GitHub LFS quotas are
    typically monthly) if the "budget" in question is a bandwidth quota
    rather than a storage quota — check which one your error refers to.
- `git lfs pull` may still fail on **old** commits/tags that reference the
  LFS objects (e.g. old release tags), for the same reason: history is
  unchanged.

## Step 4 — Verify afterward

```bash
# Confirm binaries are no longer tracked by Git:
git ls-files src-tauri/binaries/          # should print nothing

# Confirm .gitattributes no longer exists / has no LFS rules:
test -f .gitattributes && cat .gitattributes || echo "(deleted, as expected)"

# Confirm nothing under src-tauri/binaries/ shows as untracked clutter:
git status --porcelain src-tauri/binaries/   # should print nothing (ignored)

# Confirm the new script is actually tracked (it's gitignored by default —
# see the gotcha above — so double-check it made it into the commit):
git show --stat HEAD | grep fetch-binaries

# Sanity-check a clean fetch still works from a scratch checkout:
#   git clone <repo-url> /tmp/u-download-clean-check
#   cd /tmp/u-download-clean-check
#   scripts/fetch-binaries.sh
#   ./src-tauri/binaries/<your-platform>/yt-dlp --version

# Confirm CI is unblocked: push a test tag or open the workflow run for the
# next real release tag and check the "Fetch bundled binaries" step logs.
```

## Reference: what running the fetch script actually produced here

For the full verification transcript (real run + idempotent re-run output,
sizes, and the exact upstream URLs chosen for each binary/platform), see
this task's report at
`.superpowers/sdd/2026-08-24-trim-and-queue-foundation/task-14-report.md`.
