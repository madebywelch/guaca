---
name: guaca-release
description: Build, version, sign, notarize, and upload Guaca macOS releases, or refresh an existing GitHub draft from main. Use for Guaca release work, not ordinary local app installation.
---

# Guaca releases

Read [the release procedure](../../../docs/RELEASING.md) in the Guaca checkout.
It is the source of truth for commands, version files, verification, and retries.
Locate the checkout through the current workspace and its `origin`; do not rely
on a past worktree path. If this skill is installed through a symlink, resolve it
before following the relative documentation link.

## Decide what is being released

- Fetch `origin` and inspect the release and its tag before selecting a source.
  Compare its recorded commit with `origin/main` for an update request.
- Refreshing a draft keeps its unpublished version. A new release updates the
  package, Tauri, and Cargo versions together, including Cargo's lockfile entry.
  Merge that change before building from `main`.
- A live release is no longer a draft candidate. Use a new version for subsequent
  builds unless the operator explicitly requests repair of the published release.
- Carry forward the operator's authorization: a draft request stays a draft;
  publishing requires authorization to publish. Do not ask again for actions
  already authorized in the current task.

## Build and verify

Use a clean detached worktree at the recorded full SHA. Run `scripts/ci.sh` and
relevant live checks for changed integrations; record real results. Build the
universal DMG with locked dependencies, not `scripts/install.sh`, which installs
and may re-sign the operator's local app.

The local signing identity is `Developer ID Application: Robert Welch (LN79XLHFLT)`.
Verify it is still valid using `security find-identity -v -p codesigning`.
The notarization Keychain profile is `guaca-notary`. Reuse it; never read or export
its password or private key. These names are public metadata, not credentials.
If a system prompt appears, distinguish a Mac login Keychain unlock from Apple's
app-specific password setup rather than asking for a password in chat.

The desktop also needs its matching backend image. Verify anonymous multi-platform
pulls and pin `GUACA_BACKEND_IMAGE` to its digest for distribution. A signed app
with an unavailable image cannot complete local setup. A draft can document that
unresolved condition; signing is not evidence that Docker or remote-host flows work.

Submit the signed DMG with `notarytool`, retain its ID, and require acceptance.
A pending submission is not a reason to upload again. The release procedure
includes the bounded retry for an upload crash. Staple and validate the DMG and
app, then verify the app mounted from the final DMG. Create the checksum after
stapling. Keep artifacts and receipts under an ignored directory named for the
source revision.

## Upload and report

Use `gh` for the current GitHub origin. Re-read release state before mutating it.
Check existing tags resolve to the built SHA; `--target` does not retarget a tag.
Preserve previous draft assets before replacement. Upload only the verified DMG
and checksum; write notes from a file with the source, changes, backend image,
installation requirements, migration steps, and actual validation results.

Verify the remote state, source, notes, and both asset digests after upload.
Recheck `main` if the operator requested the latest commit. Report the release
URL, built SHA, absolute artifact directory, publication state, and any material
installation gaps. A rebase or edited release description does not update a binary.
