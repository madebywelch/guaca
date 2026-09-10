# macOS releases

Guaca can be distributed directly as a signed, notarized disk image. This does
not require an App Store listing or App Review. Signing identifies the publisher;
notarization is Apple's automated check, and stapling attaches its ticket for
offline verification. A signature alone does not complete the release.

The current desktop connects to a containerized backend. A release therefore
has two artifacts: the macOS download in GitHub Releases and its matching
backend image in GitHub Container Registry (GHCR). Users need Docker for **On
this Mac**, or an existing backend for **Remote host**. They do not need Node,
Rust, or a source checkout. See [Hosting](HOSTING.md#installing-after-the-hosting-changes).

## One-time setup

Use the **Robert Welch** developer team when creating the certificate. In Xcode,
open Settings > Apple Accounts > the account > Robert Welch > Manage Certificates,
then add **Developer ID Application**. Apple Development and Apple Distribution
certificates are for different distribution paths.

Confirm the certificate and its private key are available:

```sh
security find-identity -v -p codesigning
```

The release identity is `Developer ID Application: Robert Welch (LN79XLHFLT)`.
That name and team ID are public metadata, embedded in every signed download.
The private key stays in the login Keychain. Do not export it into this repository.
The developer name comes from Apple's certificate, not the app's author field.

Create an app-specific password in the personal Apple Account at
[account.apple.com](https://account.apple.com). Store it through the secure prompt
in your own Terminal, using the Apple Account that belongs to this team:

```sh
xcrun notarytool store-credentials guaca-notary \
  --apple-id YOUR_APPLE_ACCOUNT_EMAIL --team-id LN79XLHFLT
```

The password is saved in Keychain, not in a shell script, command argument, or
environment file. An existing App Store Connect API key is another supported
authentication method, but creating an App Store app record is unnecessary.

## Choose the source and version

Inspect `git remote -v`, fetch `origin`, and inspect the existing release before
writing anything remotely. This repository currently uses GitHub. A request to
refresh a draft authorizes updating that draft; it does not authorize publishing
it. If the release is already live, prepare a new version instead of replacing
its assets or moving its tag, unless the operator explicitly requests a repair.

For a new version, update these together in a version-bump PR:

- `package.json`: `version`.
- `src-tauri/tauri.conf.json`: `version`.
- `src-tauri/Cargo.toml`: the `guac` package version.
- `src-tauri/Cargo.lock`: the `guac` package entry only. Regenerate with Cargo
  and inspect the diff for unintended dependency updates.

`pnpm-lock.yaml` currently has no root package version. Keep its dependency
resolution unchanged. The backend image tag is a separate deployment input;
search `GUACA_BACKEND_IMAGE` and the default in `src-tauri/src/host.rs` rather
than assuming a desktop version bump updates it. Pin the image as described below.

Merge the version bump before building a release from `main`. A draft refresh
can retain its unpublished version. Record the full commit SHA and build it in
a clean detached worktree, leaving ongoing local work alone:

```sh
git fetch origin
GUACA_SOURCE="$(git rev-parse origin/main)"
GUACA_TREE="$(mktemp -d /private/tmp/guaca-release.XXXXXX)/source"
git worktree add --detach "$GUACA_TREE" "$GUACA_SOURCE"
cd "$GUACA_TREE"
GUACA_VERSION="$(node -p 'JSON.parse(require("fs").readFileSync("package.json", "utf8")).version')"
GUACA_TAG="v$GUACA_VERSION"
```

Inspect the remote tag if it already exists. `gh release --target` does not move
an existing tag. Its peeled commit must equal `GUACA_SOURCE`; otherwise resolve
the mismatch before uploading. Recheck `origin/main` before upload when the
request is specifically for latest `main`. If it advanced, rebuild or explicitly
report the pinned revision instead of describing it as current.

## Build and sign

Build the intended commit from a clean checkout. About embeds that commit;
uncommitted files cause it to show a `-dirty` suffix. Keep both lockfiles and use
the pinned pnpm version from `package.json`.

First build, test, and publish the backend from that same source commit:

1. Run `./scripts/ci.sh` and `./scripts/image.sh`. The latter checks the local
   image's health, authentication, bundled tools, and restart persistence.
2. Publish the root `Dockerfile` as a multi-platform image for `linux/arm64`
   and `linux/amd64`, supplying the full source SHA as `GUACA_COMMIT`,
   `GUACA_RELEASE=1`, and the package version as `GUACA_VERSION` build arguments. Use
   `ghcr.io/madebywelch/guaca/guacad` with a version tag for the release.
3. In the GitHub package settings, make that package public. A public source
   repository does not automatically make its package public. Verify image
   pulls work without saved registry credentials on both architectures.
4. Record the multi-platform image's immutable digest and export
   `GUACA_BACKEND_IMAGE=ghcr.io/madebywelch/guaca/guacad@sha256:THE_DIGEST` in the
   shell that will build the desktop. Replace `THE_DIGEST` with the real digest.

Without that variable, the app defaults to
`ghcr.io/madebywelch/guaca/guacad:0.1.0`. Do not rely on this fallback unless it
is published and matches the desktop. A local `guacad:...` image works only on
the builder's Mac. `scripts/release-candidate.sh` checks a candidate but builds
only an app bundle; use the command below for the universal DMG.

```sh
: "${GUACA_BACKEND_IMAGE:?Set the published backend image digest first}"
export GUACA_RELEASE=1
export GUACA_COMMIT="$GUACA_SOURCE"
pnpm install --frozen-lockfile
rustup target add aarch64-apple-darwin x86_64-apple-darwin
./scripts/ci.sh
APPLE_SIGNING_IDENTITY='Developer ID Application: Robert Welch (LN79XLHFLT)' \
  pnpm tauri build --target universal-apple-darwin --bundles app,dmg -- --locked
```

The universal binary contains both Apple Silicon and Intel code. Tauri enables
the hardened runtime by default and signs with the Keychain identity. No private
key, password, or signing identity is needed in `tauri.conf.json`, so contributors
can still build without a developer account. With no notarization credentials in
the build environment, Tauri reports that notarization was skipped; the next step
performs it explicitly using Keychain.

The app is in `src-tauri/target/universal-apple-darwin/release/bundle/macos/`.
The disk image is in the sibling `dmg/` directory. Verify the actual output before
uploading it:

```sh
GUACA_APP=src-tauri/target/universal-apple-darwin/release/bundle/macos/Guaca.app
GUACA_DMG="src-tauri/target/universal-apple-darwin/release/bundle/dmg/Guaca_${GUACA_VERSION}_universal.dmg"
codesign --verify --deep --strict --all-architectures --verbose=2 "$GUACA_APP"
codesign --display --verbose=4 "$GUACA_APP"
lipo -archs "$GUACA_APP/Contents/MacOS/guac"
codesign --verify --verbose=2 "$GUACA_DMG"
hdiutil verify "$GUACA_DMG"
```

The app's
signature must name Robert Welch, show team `LN79XLHFLT`, include a secure
timestamp, and have the `runtime` flag. Do not use `scripts/install.sh` to prepare
a release: it replaces the installed app and re-signs locally, with an ad-hoc
signature by default.

## Notarize and verify

Submit the signed disk image. Apple's service checks the app inside it too:

```sh
xcrun notarytool submit "$GUACA_DMG" --keychain-profile guaca-notary \
  --no-progress --output-format json --wait --timeout 10m
```

Proceed only when the submission status is **Accepted**. Preserve the submission
ID. A first submission can take substantially longer than subsequent ones. If
the connection ends before completion, query the existing submission instead of
uploading it again:

```sh
xcrun notarytool info SUBMISSION_ID --keychain-profile guaca-notary
xcrun notarytool log SUBMISSION_ID --keychain-profile guaca-notary
```

A timeout is not rejection. Continue checking the same submission. If
`notarytool` crashes after returning an ID but before confirming upload, check
that ID first. A local SIGBUS occurred during the initial upload; one retry with
progress disabled and S3 acceleration disabled succeeded:

```sh
xcrun notarytool submit "$GUACA_DMG" --keychain-profile guaca-notary \
  --no-progress --no-s3-acceleration --output-format json --wait --timeout 10m
```

Use this retry only for an apparent upload/client failure, not an ordinary
`In Progress` response after a confirmed upload. If the retry also fails, retain
both IDs and diagnose the failure instead of submitting in a loop. Preserve the
accepted submission's JSON log locally, and inspect any reported issues.

After acceptance, attach the ticket and check Gatekeeper:

```sh
xcrun stapler staple "$GUACA_DMG"
xcrun stapler validate "$GUACA_DMG"
xcrun stapler staple "$GUACA_APP"
xcrun stapler validate "$GUACA_APP"
spctl --assess --type open --context context:primary-signature --verbose=2 "$GUACA_DMG"
spctl --assess --type execute --verbose=2 "$GUACA_APP"
xcrun syspolicy_check distribution "$GUACA_APP"
shasum -a 256 "$GUACA_DMG"
```

Mount the final DMG read-only and run the signature, architecture, and Gatekeeper
checks against the `Guaca.app` inside it, then detach the volume. Stapling the
separate build-directory app does not modify the app already packaged in the DMG.
Do not repack the DMG after notarization; changing it requires a new submission.

The app assessment should report `accepted` and `source=Notarized Developer ID`.
Test the final disk image as a real download on a clean Mac or VM: open it in
Finder, drag Guaca into Applications, and launch it with Gatekeeper enabled.
Confirm first-run settings work without cloning the repo, Node, or Rust. Optional
coding harnesses are included in the backend image; their sign-ins must be set
up on the backend. Test **On this Mac** with Docker and **Remote host** against
a matching host. A universal build still needs a launch check on both Intel
and Apple Silicon.

## Upload manually to GitHub Releases

No GitHub Action or GitHub-hosted Apple credential is required for a local build.
Sign and notarize on the Mac, then upload the finished files in the browser.

Create a checksum file after stapling, beside the DMG:

```sh
(
  cd "$(dirname "$GUACA_DMG")"
  GUACA_FILENAME="$(basename "$GUACA_DMG")"
  shasum -a 256 "$GUACA_FILENAME" > "$GUACA_FILENAME.sha256"
  shasum -a 256 -c "$GUACA_FILENAME.sha256"
)
```

1. Open the repository's **Releases** page and edit the intended draft, or
   choose **Draft a new release**.
2. Select a version tag pointing to the exact commit used for both builds.
   Do not let a moving `main` select a different commit for an older binary.
   Keep the release tag and the versions in `package.json`,
   `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json` consistent; update
   the lockfile's package version when changing the Cargo version.
3. Attach the final `.dmg` and `.dmg.sha256` in the release assets box.
4. In the notes, record the source commit, backend image digest, macOS
   architectures, Docker/remote-host requirements, and any migration steps.
5. Save the draft while testing. Publish after the download and first-run
   checks pass, then download the public asset and check its checksum.

## Create or refresh a draft with the CLI

Use the repository derived from `origin`, and read the current state first:

```sh
GUACA_REPO=madebywelch/guaca
git remote -v
gh release view "$GUACA_TAG" --repo "$GUACA_REPO" \
  --json isDraft,targetCommitish,body,assets,url
```

For a new release, confirm that it is absent rather than treating an auth or
network failure as absence. Write the description to a local Markdown file and
set `GUACA_NOTES` to that path. Use actual validation results and note untested
install flows or unavailable backend images. Do not copy old test counts or
notarization IDs. Create the release with an exact source commit:

```sh
gh release create "$GUACA_TAG" "$GUACA_DMG" "$GUACA_DMG.sha256" \
  --repo "$GUACA_REPO" --draft --target "$GUACA_SOURCE" \
  --title "Guaca v$GUACA_VERSION" --notes-file "$GUACA_NOTES"
```

For a refresh, first confirm `isDraft: true`, preserve the previous files and
metadata, and finish signing and validating the replacement before changing the
draft. Re-read its state immediately before upload to catch publication or edits
made while the build was running. Replace only the intended assets:

```sh
gh release upload "$GUACA_TAG" "$GUACA_DMG" "$GUACA_DMG.sha256" \
  --repo "$GUACA_REPO" --clobber
gh release edit "$GUACA_TAG" --repo "$GUACA_REPO" --draft \
  --target "$GUACA_SOURCE" --title "Guaca v$GUACA_VERSION" \
  --notes-file "$GUACA_NOTES"
```

`--clobber` deletes the old asset before uploading the new one. If either upload
fails, restore or retry the intended files and verify the full pair before
reporting success. After any write, use `gh release view` again and verify the
expected draft state, source SHA, notes, filenames, sizes, and uploaded asset
SHA-256 digests. Download and hash the assets if GitHub does not return digests.

Keep the final stapled DMG, checksum, notes, source SHA, backend digest, build
logs, and notarization receipt in an ignored directory such as
`src-tauri/target/releases/<source-sha>/` before removing the build worktree.
Only the DMG and checksum belong in release assets. Report the absolute local
artifact directory and release URL to the operator.

Publish only within the operator's authorized scope. After publication, verify
the public download and checksum and that the remote tag resolves to the built
commit. Future fixes use a new version; do not silently overwrite a live build.

Rebasing the source does not update a DMG already on disk. A binary built
before the container-host changes is an older release even if its filename
still says `0.1.0`. Rebuild and notarize again to ship the current application.

Upload only the verified disk image and its SHA-256 checksum as release assets,
and identify the source commit in the release. Inspect `git remote -v` first and
publish through the repository's origin. Publish the corresponding source from
an exact source tag alongside the binaries.
Do not advertise a download before the asset exists and its Gatekeeper checks pass.

Local signing keeps credentials off CI entirely. If release signing later moves
to CI, store the encrypted certificate export and notarization credentials in
protected secrets, available only to trusted release jobs. Pull request code must
never run with those secrets. Base64 is encoding, not encryption. The ignored key
file extensions are a guard against mistakes, not a substitute for reviewing what
is committed or uploaded.

References: [Apple Developer ID](https://developer.apple.com/developer-id/),
[Apple's notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow),
and [Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/).
For uploading and package access, see
[GitHub release management](https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository)
and [GitHub Container Registry](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry).


## Release checks in browser and desktop

Every published stable release also carries `guaca-release.json`. Generate it
from the same clean checkout after recording the image digest:

```sh
GUACA_BACKEND_IMAGE=ghcr.io/madebywelch/guaca/guacad@sha256:THE_DIGEST \
  node scripts/release-manifest.mjs /path/to/artifacts/guaca-release.json
```

The generator verifies the frontend, Cargo and desktop versions agree and
refuses prereleases, mutable image tags, or an invalid API range. Upload this
file alongside the verified DMG and checksum before publishing the release.
Do not publish the manifest until both artifacts have passed verification.
The public stable feed is GitHub's
`/madebywelch/guaca/releases/latest/download/guaca-release.json`; verify an
anonymous download returns the generated file after publication. A release
without this asset reports "could not check", not "up to date".

`release-protocol.json` is shared by Rust and the frontend. Increment its API
generation for an incompatible contract change and set the client-supported
range deliberately. `GUACA_RELEASE=1` marks an official build; source installs
remain unverified/development builds even when their package version matches.
The Rust build rejects official releases without a full source SHA, and rejects
an official desktop without a pinned GHCR image digest. Build and verify the
same metadata on both architectures.

The feed is advisory. It cannot supply an executable update target: the native
manager installs only its embedded image. Installing directly from a future
feed requires signed metadata and verification in the update manager first.
There is no automatic desktop download or remote deployment in this version.
