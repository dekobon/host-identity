# Cutting a release

This is the step-by-step procedure for releasing `host-identity`.
The developer-guide section on releases is the terse reference; this
document is the walkthrough — what to do, in what order, and what to
check when something looks wrong.

The pipeline is defined in
[`.github/workflows/release.yml`](../.github/workflows/release.yml).
Everything downstream of `git push --tags` is automated.

## What the release pipeline does

One push of a `v*` tag runs this end-to-end:

1. **preflight** — validates the tag, checks `Cargo.toml` version
   parity, confirms `minisign.pub` is not the placeholder, regenerates
   man pages and fails if they drift, extracts the matching
   `CHANGELOG.md` section as release notes.
2. **build** — cross-compiles `hostid` for nine targets (Linux
   gnu/musl x86_64+aarch64, FreeBSD x86_64, macOS x86_64+aarch64,
   Windows x86_64+aarch64). Strips binaries, captures debug symbols,
   produces per-target `.tar.gz` / `.zip` archives.
3. **package-*** — builds `.deb`, `.rpm`, `.apk`, and FreeBSD `.pkg`
   artefacts from the staged binaries.
4. **smoke-*** — installs each package inside the appropriate
   container/VM (Ubuntu 22.04/24.04, Debian 12, Rocky 9, Fedora,
   Amazon Linux 2023, Alpine 3.20, FreeBSD 14, macOS 13+latest,
   Windows) and asserts `hostid --version` matches the tag.
5. **sign-attest** — flattens every artefact into `release/`,
   generates CycloneDX SBOMs, computes `SHA256SUMS`, signs it with
   minisign, and attaches SLSA build provenance.
6. **publish** — creates/updates the GitHub Release, attaches every
   artefact + `SHA256SUMS` + `SHA256SUMS.minisig`, and (for non
   pre-releases) pushes the Homebrew formula, Scoop manifest, and
   winget PR.
7. **verify** — downloads the published `musl` tarball back out of
   the release, verifies the minisign signature, checksum, and SLSA
   provenance.

If any stage fails, nothing downstream runs. `publish` is the only
job that mutates anything outside this repo.

## Prerequisites (one-time setup)

You only need to do this once per project, but verify each item
before your first real release.

### Repository secrets

Configure these under **Settings → Secrets and variables → Actions**:

| Secret                   | Purpose                                                            |
| ------------------------ | ------------------------------------------------------------------ |
| `MINISIGN_SECRET_KEY`    | minisign secret-key file contents (base64 body + comment lines) used to sign `SHA256SUMS` |
| `MINISIGN_PASSWORD`      | Password for that key                                              |
| `ALPINE_ABUILD_KEY_PRIV` | abuild RSA private key (Alpine `.apk` signing)                     |
| `ALPINE_ABUILD_KEY_PUB`  | Matching public key                                                |
| `HOMEBREW_TAP_TOKEN`     | Fine-grained PAT with write access to `dekobon/homebrew-host-identity` |
| `SCOOP_BUCKET_TOKEN`     | Fine-grained PAT with write access to `dekobon/scoop-bucket`       |
| `WINGET_TOKEN`           | PAT used by `winget-releaser` to open the microsoft/winget-pkgs PR |

If `HOMEBREW_TAP_TOKEN` or `SCOOP_BUCKET_TOKEN` is missing, those
steps log a message and skip without failing the release. If
`WINGET_TOKEN` is missing, the winget step **fails** — the
`winget-releaser` action errors out when its token input is empty.
Either configure all three, or remove the winget step for
releases you don't plan to ship to winget.

### Minisign key

`minisign.pub` at the repo root must be a real public key, not the
committed placeholder. The preflight job greps for the placeholder
comment and aborts if it's still present.

To create a fresh key:

```bash
minisign -G -p minisign.pub -s minisign.key
```

Commit `minisign.pub`. Paste the contents of `minisign.key` into
`MINISIGN_SECRET_KEY` and its password into `MINISIGN_PASSWORD`.
Keep `minisign.key` out of the repo.

### External repos

Releases (for stable versions) push to:

- `dekobon/homebrew-host-identity` — Homebrew tap
- `dekobon/scoop-bucket` — Scoop bucket
- `microsoft/winget-pkgs` — winget manifests (via PR)

Both tap and bucket repos must exist and accept the configured PAT.

## Pre-release checklist

Before tagging, on `main`:

- [ ] All intended changes are merged and CI is green.
- [ ] `Cargo.toml` `[workspace.package] version` is bumped to the
      target version (e.g. `0.2.0`).
- [ ] `crates/host-identity/CHANGELOG.md` has a `## [x.y.z]` section
      with the release notes. The header must match the tag exactly,
      minus the leading `v`. Move entries out of `## [Unreleased]`
      into the new section.
- [ ] `cargo xtask` has been run and any `man/*.1` diffs are committed
      — the preflight job will fail the release otherwise.
- [ ] `cargo test --workspace` passes locally.
- [ ] `minisign.pub` is a real key (run
      `grep '^untrusted comment: placeholder' minisign.pub` — it
      should print nothing).

Commit and push these changes. The final commit on `main` before
tagging should be the release-prep commit.

## Cutting a stable release

Pick a semver version (e.g. `0.2.0`). The tag is the version prefixed
with `v`.

```bash
# From a clean main checkout at the release-prep commit:
git tag -a v0.2.0 -m "v0.2.0"
git push origin v0.2.0
```

That's it — the push of the tag triggers `release.yml`. Watch it in
the Actions tab:

```bash
gh run watch
# or
gh run list --workflow=Release
```

Expect ~20–40 minutes end-to-end (the FreeBSD VM and cross-compiled
smoke tests are the slowest stages).

## Cutting a pre-release

Pre-release tags match `vX.Y.Z-<suffix>` (e.g. `v0.2.0-rc.1`,
`v0.2.0-beta.2`). The preflight classifier sets `prerelease=true`,
which:

- Marks the GitHub Release as a pre-release.
- Skips the Homebrew tap, Scoop bucket, and winget PR steps.

Use this for any version that should not reach package managers.
Signed artefacts, SBOMs, and SLSA provenance still publish normally,
so a pre-release is a full test of everything except the external
pushes.

## Rehearsing a release (dry run)

Use this to exercise the pipeline without creating a real version.

**Preferred:** re-run the workflow against an existing tag via
**Actions → Release → Run workflow**, typing the tag name in the
`tag` input. This requires no new commits or tags and is the cleanest
way to retry a stage.

**Alternative:** create a throwaway pre-release tag. This path
requires a matching changelog section on the tagged commit — the
preflight checks out the tag ref and greps the changelog there, so a
runtime-only flag is not enough. Commit a placeholder entry, tag,
push, then revert the placeholder once the rehearsal is done:

```bash
# 1. On main, append `## [0.0.0-test1]` with a one-line body to
#    crates/host-identity/CHANGELOG.md and commit.
# 2. Tag and push:
git tag -a v0.0.0-test1 -m "pipeline rehearsal"
git push origin v0.0.0-test1
# 3. After the rehearsal, revert the placeholder commit on main.
```

The pipeline is designed to be idempotent on re-run:

- `softprops/action-gh-release` overwrites existing release assets.
- Tap and bucket pushes are no-ops when the rendered files are
  unchanged.
- `winget-releaser` deduplicates by version.

Clean up after a rehearsal by deleting the tag and the GitHub
Release:

```bash
gh release delete v0.0.0-test1 --cleanup-tag --yes
```

## Monitoring a live release

While the workflow runs:

```bash
gh run list --workflow=Release --limit 1
gh run view --log-failed   # on any failure
```

Common failure signatures and what they mean:

| Failure                                         | Cause                                                                 |
| ----------------------------------------------- | --------------------------------------------------------------------- |
| `Tag version (X) != host-identity-cli Cargo.toml version (Y)` | You forgot to bump `Cargo.toml` before tagging.                      |
| `CHANGELOG.md has no section for [x.y.z]`       | The changelog heading is missing, mistyped, or not on `main`.         |
| `Committed man pages drift from clap schema`    | Run `cargo xtask` locally and commit `man/`.                          |
| `minisign.pub is still the committed placeholder` | Set up the minisign key per the prerequisites above.                |
| Smoke test `hostid --version` mismatch          | The binary built from the tag reports a different version. Usually the tag points at a commit older than the version bump, or a previous run left overlapping assets on the release that `softprops/action-gh-release` picked up. |
| `MINISIGN_SECRET_KEY not configured`            | Repo secret missing.                                                  |

## Post-release verification

The pipeline's own `verify` job downloads the musl tarball from the
published Release and re-runs minisign + SLSA verification. That
covers the critical path automatically.

Verify manually if you want extra assurance:

```bash
# From a fresh directory:
TAG=v0.2.0
VERSION=0.2.0
gh release download "$TAG" -R dekobon/host-identity \
  -p "hostid-${VERSION}-x86_64-unknown-linux-musl.tar.gz" \
  -p SHA256SUMS -p SHA256SUMS.minisig

# Fetch minisign.pub from the tag, not main — if the key was rotated
# after this release, main has a different key and verification fails.
curl -fsSLO "https://raw.githubusercontent.com/dekobon/host-identity/${TAG}/minisign.pub"
minisign -Vm SHA256SUMS -p minisign.pub
grep "hostid-${VERSION}-x86_64-unknown-linux-musl.tar.gz" SHA256SUMS | sha256sum -c
gh attestation verify "hostid-${VERSION}-x86_64-unknown-linux-musl.tar.gz" \
  -R dekobon/host-identity
```

Check that the downstream package managers updated:

- Homebrew tap: new commit on `dekobon/homebrew-host-identity`
  bumping `Formula/hostid.rb`.
- Scoop bucket: new commit on `dekobon/scoop-bucket` bumping
  `bucket/hostid.json`.
- winget: a PR opened against `microsoft/winget-pkgs` by
  `winget-releaser`. This one needs human review on the winget side
  and may take days to merge — that is out of our control.

## Fixing a broken release

The pipeline fails *before* publish on any preflight, build, package,
or smoke error, so a broken release almost never reaches users.

If publish itself partially succeeds (e.g. GitHub Release created but
tap push failed), the fix is usually to re-run the workflow against
the same tag via **Actions → Run workflow**. The pipeline is
idempotent (see "Rehearsing" above).

If you need to pull a release entirely:

```bash
gh release delete vX.Y.Z --cleanup-tag --yes
```

Then fix the underlying issue, bump to `vX.Y.(Z+1)`, and re-tag.
**Do not re-use a published version number** — Homebrew/Scoop/winget
users may have already cached the old artefacts.

If the tap or bucket needs a manual revert, clone the repo and revert
the offending commit by hand. The release bot only writes a single
file per release, so reverts are straightforward.

## Rotating the minisign key

1. Generate a new keypair: `minisign -G -p minisign.pub.new -s minisign.key.new`.
2. Replace `minisign.pub` with the new public key and commit it.
3. Update `MINISIGN_SECRET_KEY` and `MINISIGN_PASSWORD` secrets with
   the new values.
4. Cut a new release — its `SHA256SUMS.minisig` will be signed with
   the new key, self-documenting the rotation.

Users verifying an older release still need the old `minisign.pub`
from that release's tagged commit.

## Known gaps

Documented in
[`docs/developer-guide.md`](developer-guide.md#deferred-known-gaps) —
no macOS notarization, no Windows code signing, no MSI/Chocolatey,
no `aarch64-unknown-freebsd` (Rust tier 3). None of these block a
release; they just mean some users hit platform-specific friction
(Gatekeeper, SmartScreen) on first run.
