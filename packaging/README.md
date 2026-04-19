# packaging/

Templates consumed by `.github/workflows/release.yml` on every `v*`
tag. Every template uses `@@TOKEN@@` placeholders that the release
workflow substitutes at build time:

| Token                  | Meaning                                   |
| ---------------------- | ----------------------------------------- |
| `@@VERSION@@`          | `${GITHUB_REF_NAME#v}` (e.g. `0.1.0`)     |
| `@@TARGET@@`           | Rust target triple                        |
| `@@ARCH@@`             | Alpine arch list (`x86_64` / `aarch64`)   |
| `@@SHA256_*@@`         | SHA-256 of the matching release tarball   |
| `@@SHA512@@`           | SHA-512 (Alpine)                          |

| File                              | Consumer                           |
| --------------------------------- | ---------------------------------- |
| `alpine/APKBUILD.in`              | `abuild -r` in Stage 2             |
| `freebsd/+MANIFEST.in`            | `pkg create -M` in Stage 2         |
| `freebsd/port/`                   | Published as-is for ports-tree PRs |
| `homebrew/hostid.rb.tmpl`         | Pushed to `homebrew-host-identity` |
| `scoop/hostid.json.in`            | Pushed to `scoop-bucket`           |
