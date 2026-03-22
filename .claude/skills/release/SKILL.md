---
name: release
description: Push the prepared orion-sdr-view release tag and publish to crates.io. Run release-prep first.
disable-model-invocation: true
allowed-tools: Bash
argument-hint: <version>  (e.g. 0.0.2)
---

Publish the orion-sdr-view release for version $ARGUMENTS.

VERSION = $ARGUMENTS  (without the leading "v")
TAG = v$ARGUMENTS

This skill assumes `/release-prep VERSION` has already been run successfully:
the version bump commit exists locally and the signed tag TAG exists locally.

## Step 1 — Verify preconditions

- Confirm the local tag TAG exists: `git tag -l TAG`
- Confirm the tag signature is valid: `git tag -v TAG`
- Confirm the commit the tag points to is ahead of origin/main:
  `git log origin/main..TAG --oneline`

If any check fails, stop and tell the user what is missing.

## Step 2 — Push commit and tag

Push in this order (commit first so the tag's target exists on the remote):

```
git push
git push origin TAG
```

## Step 3 — Publish to crates.io

```
cargo publish
```

If publish fails with "already uploaded", the version is already on crates.io —
treat this as success and continue.

## Step 4 — Report

Tell the user:
- Commit and tag TAG have been pushed to GitHub
- crates.io publish result (success or already-uploaded)
- Link to the GitHub repo: https://github.com/skynavga/orion-sdr-view
- Link to the crates.io release: https://crates.io/crates/orion-sdr-view/VERSION
