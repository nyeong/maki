# Maki release contract

The canonical Maki source is the anonymously readable
[Forgejo repository](https://git.eska.nyeong.me/nyeong/maki). Release tooling
must not require a maintainer checkout, private remote, or machine-local path.

## Versions and compatibility

The Maki CLI and LSP ship from this workspace with one semantic version. The
root package, internal crates, lock file, Nix package, `maki --version --json`,
and the LSP `serverInfo.version` must agree on that version.

The Tree-sitter grammar and Zed extension are independently versioned. Maki
does not pin either downstream project. A Zed extension release owns its
minimum supported Maki version and its exact immutable grammar revision.

The supported Maki distribution is the repository's Nix package. Workspace
crates are implementation units and are not published independently to a
Cargo registry.

## Source provenance

A public release is identified by all of the following:

- a `maki-vMAJOR.MINOR.PATCH` tag in the canonical repository;
- the full 40-character commit reached by that tag;
- a matching workspace and Nix package version; and
- a dated changelog section for the same version.

Release notes record both the tag and full commit. Build inputs come from that
commit and the committed `Cargo.lock`. The Nix-built CLI reports the commit as
`source_revision` in `maki --version --json`. Moving a release tag or rebuilding
from a different commit does not create the same release.

## Release-candidate gate

Run the canonical repository check before creating a tag:

```bash
bash scripts/ci/check-maki.sh
python3 scripts/ci/check-release-metadata.py --release .
```

The gate checks package tests and formatting as well as license presence,
public source metadata, version agreement, internal dependency versions, and
the absence of private or machine-local release sources. Release mode also
requires a dated changelog section and source tag for the package version.
