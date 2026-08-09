---
name: release
description: Push the prepared orion-sdr-view release tag and publish to crates.io. Run release-prep first.
allowed-tools: Bash
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

## Step 4 — Report

Tell the user:

- Commit and tag TAG have been pushed to GitHub
- crates.io publish result (success or already-uploaded)
- Link to the GitHub repo: <https://github.com/skynavga/orion-sdr-view>
- Link to the crates.io release: <https://crates.io/crates/orion-sdr-view/VERSION>
