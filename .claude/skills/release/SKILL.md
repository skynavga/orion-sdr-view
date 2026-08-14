---
name: release
description: Push the prepared orion-sdr-view release tag, publish to crates.io, and cut the GitHub release. Run release-prep first.
allowed-tools: Bash, Read, Write
argument-hint: <version>  (e.g. 0.0.2)
---

# orion-sdr-view release

Publish the orion-sdr-view release for version $ARGUMENTS.

VERSION = $ARGUMENTS  (without the leading "v")
TAG = v$ARGUMENTS

This skill assumes `/release-prep VERSION` has already been run successfully:
the version bump commit exists locally and the signed tag TAG exists locally.

## Step 1 — Verify preconditions

`/release-prep` merges the release PR into `main` and pulls **before** it
creates the tag, so by the time this runs the tagged commit is already on
`origin/main`. Do **not** check `git log origin/main..TAG` — that range is
empty on every successful release, so it can only ever produce a false alarm.
What actually needs confirming is that the tag exists, is signed, points at
`main`, and has not already been pushed.

```sh
git fetch origin --tags
git tag -l TAG                                  # exists locally
git tag -v TAG                                  # signature is good
git ls-remote --tags origin refs/tags/TAG       # MUST be empty — not yet pushed
git merge-base --is-ancestor TAG origin/main    # tagged commit is on main
git rev-parse HEAD origin/main                  # local main matches the remote
```

Read the results rather than trusting exit codes alone:

- `git ls-remote` printing a ref means the tag is **already published**. Stop —
  re-tagging a released version is the one genuinely destructive mistake here.
  Check whether `cargo publish` also ran (step 3) before doing anything else.
- `git merge-base --is-ancestor` failing means release-prep did not complete its
  merge. Stop and say so; pushing the tag would publish a commit that is not on
  `main`.

If any check fails, stop and tell the user what is missing.

## Step 2 — Push commit and tag

```sh
git push
git push origin TAG
```

`git push` is normally a no-op — release-prep already merged and pulled — but
it is kept as a safety net for the case where the merge landed locally and the
push did not. The order matters if it ever does something: the tag's target
must exist on the remote before the tag references it.

## Step 3 — Publish to crates.io

```sh
cargo publish
```

If publish fails with "already uploaded", the version is already on crates.io —
treat this as success and continue.

## Step 4 — Cut the GitHub release

### Establish the boundary

Cutting the release was a manual step until 0.0.26, so several tags never got
one — v0.0.19 through v0.0.21 are covered by v0.0.25's predecessor, and
v0.0.15/v0.0.16 by none at all. Now that this step exists that should stop
happening, but the notes must still cover **everything since the last release
that exists**, which is not necessarily the previous tag:

```sh
gh release list --limit 1
gh release view --json tagName -q .tagName
```

Call the result PREV_TAG. If there is no prior release at all, use the repo's
first commit as the boundary.

### Gather the delta

- Commits: `git log PREV_TAG..TAG --oneline`
- Merged PRs in the range: the merge commits in that log name them
  (`Merge pull request #NN from ...`). Read each with `gh pr view NN` for the
  problem statement and any measured numbers.
- CHANGELOG: read the `## [x.y.z]` sections for **every** version in
  `(PREV_TAG, TAG]`, not only NEW_VERSION.
- Scale of the change: `git diff PREV_TAG..TAG --stat | tail -1`
- Test counts, which the notes quote: `cargo test --release --features gui`
  and `cargo test --release --no-default-features` report different totals, and
  saying so is the point — see the CI note in the previous entries.

### Match the house style

Read the previous two releases before drafting — they are the style reference,
and it differs from orion-sdr's:

```sh
gh release view PREV_TAG
```

Title: `vVERSION — <short phrase naming what changed>`, e.g.
`v0.0.24 — COFDM tuning knobs, configurable viewport span`. Describe the
change, not the version. Use an em dash, not a hyphen.

Body, in this order (omit a section only when it would be empty):

- **An opening paragraph**, BLUF, beginning literally `Changes since
  **PREV_TAG**.` followed by the theme — these releases each have one, and
  saying it in a sentence is what makes the bullet list readable.
- **A bolded breaking-status paragraph**: `**Not a breaking change.**` or
  `**This is a breaking config change** — see …`, then what a user will
  nonetheless notice. State it either way; silence reads as "unknown".
- **Emoji section headings**, drawn from those already in use:
  `## ⚠️ Breaking`, `## ✨ Added`, `## 🔧 Changed`, `## 🐛 Fixed`,
  `## 🧪 Testing`, `## 📏 Measured`, `## 📖 Documentation`. Order them by
  what matters most in *this* release, not alphabetically — Breaking always
  first when present, Documentation last.
- Bullets each open with a **bolded claim** and then the reasoning behind it.
  Prefer measured numbers to adjectives, and quote the same figures the
  CHANGELOG does. Include notable *rejected* alternatives where they explain
  the design.

Wrap prose at 80 columns, as the existing bodies do.

**There is no closing boilerplate line.** The last section's final bullet ends
the body. Do not append a `Co-Authored-By:` trailer, a "Generated with Claude
Code" line, or any other attribution footer — the same rule as for commit
messages and PR descriptions.

### Create it

Write the body to a scratch file outside the repo, then create the release
against the already-pushed tag:

```sh
gh release create TAG --title "vVERSION — <phrase>" \
  --notes-file /tmp/orion-sdr-view-release-VERSION.md --latest
```

Then verify and clean up:

```sh
gh release view TAG
rm /tmp/orion-sdr-view-release-VERSION.md
```

If a release for TAG already exists, do not create a second one — show the user
the existing release and ask whether to edit it (`gh release edit TAG`).

## Step 5 — Report

Tell the user:

- Commit and tag TAG have been pushed to GitHub
- crates.io publish result (success or already-uploaded)
- Which tag range the GitHub release notes cover (PREV_TAG..TAG)
- Link to the GitHub release:
  <https://github.com/skynavga/orion-sdr-view/releases/tag/TAG>
- Link to the crates.io release: <https://crates.io/crates/orion-sdr-view/VERSION>
