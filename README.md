# Maki

Maki is a line-first lightweight markup language and personal wiki runtime for
`.maki` files.

It is built around a project directory, a small `maki.toml` manifest, and plain
text notes that can be served as HTML.

The canonical source is the anonymously readable
[Forgejo repository](https://git.eska.nyeong.me/nyeong/maki). The CLI and LSP
share one release version; see the [release contract](RELEASES.md) and
[changelog](CHANGELOG.md) for provenance and release history.

## Quick Start

From this repository:

```bash
nix run .#maki -- serve .
```

Then open <http://127.0.0.1:4000>. This repository is itself a Maki project:
`maki.toml` sets `source = "docs"` and `home = "index"`.

To make your own project:

```text
my-notes/
  maki.toml
  docs/
    index.maki
    notes/today.maki
```

`maki.toml`:

```toml
[project]
title = "My Notes"
source = "docs"
home = "index"
```

`docs/index.maki`:

```maki
--^ title: Home

= Home

See [[notes/today]].
```

Run it with:

```bash
maki serve my-notes
```

## Commands

```bash
maki --version
maki --version --json
maki serve .
maki build docs/index.maki > index.html
maki lsp
maki serve --git https://example.invalid/maki.git --branch main --state-dir /var/lib/maki/docs
maki serve . --metrics 127.0.0.1:4041
```

`maki serve <path>` walks upward from `<path>` to find `maki.toml`. If `<path>`
is the project root or inside the configured source root, it serves that source
root. Otherwise, it serves `<path>` as a plain note directory.

`maki build <file>` uses the same project discovery. Files inside the configured
source root get project-aware link resolution; other files render as standalone
HTML.

`maki lsp` starts the stdio language server for editor integration.

`maki --version --json` prints a stable machine-readable object containing the
CLI name, package version, and source revision. Nix builds from a clean commit
report its 40-character revision; direct or dirty development builds report a
null revision. The LSP initialize response reports the same package version
through `serverInfo.version`.

## Configuration

The project manifest is `maki.toml` at the project root. See
[docs/maki-toml.maki](docs/maki-toml.maki) for the full configuration
contract, including `title`, `source`, and `home`.

## Syntax

Common `.maki` building blocks:

```maki
--^ title: Example

= Heading

Paragraph with a [[note link]], [reference link][], and <https://example.com>.

[reference link]: <https://example.com>

- [ ] Todo item
- [x] Done item

: code line
```

Internal links can select a root document (`[[/path]]`), a child document
(`[[+child]]`), a heading (`[[#Heading]]`), or a document-local explicit ID
(`[[@block-id]]`). These selectors can be combined, for example
`[[/plans/job-search@checklist]]`.

See [docs/maki-syntax.maki](docs/maki-syntax.maki) for the syntax source of
truth.

## Documentation

- [Getting started](docs/getting-started.maki)
- [Project configuration](docs/maki-toml.maki)
- [Web server and routes](docs/web.maki)
- [Language server](docs/lsp.maki)
- [Documentation index](docs/index.maki)

## Web Routes

- `/`: redirect to the configured home note.
- `/<note>`: rendered note page.
- `/<note>/`: direct subdocument index for the note, including an empty state.
- `/<note>.maki`: raw source text.
- `/@/`: meta index.
- `/@/recents`: recently modified notes.
- `/@/sitemap`: human-readable sitemap.
- `/@/diagnostics`: project diagnostics.
- `/@/dates`: date index.
- `/.maki/search`: note, heading, explicit ID, and file search.
- `/.maki/project-index.json`: versioned project analysis.
- `/.maki/search-index.json`: search entries.

## Deployment

`serve --git` keeps a checkout of a configured branch and periodically fetches
updates. Put `maki.toml` at the repository root and set its `source` when notes
live in a subdirectory. `--metrics HOST:PORT` enables a separate Prometheus
listener that serves `GET /metrics`.

The NixOS module supports multiple named local or Git-backed targets:

```nix
{
  inputs.maki.url = "git+https://example.invalid/maki.git";

  outputs =
    { maki, nixpkgs, ... }:
    {
      nixosConfigurations.server = nixpkgs.lib.nixosSystem {
        modules = [
          maki.nixosModules.default
          {
            services.maki = {
              enable = true;
              targets.docs = {
                git.url = "https://example.invalid/notes.git";
                port = 4000;
                metrics.port = 4041;
                openFirewall = true;
              };
            };
          }
        ];
      };
    };
}
```

Git targets default to `branch = "main"`, `fetchInterval = "60s"`, and
`stateDir = "/var/lib/maki/<target-name>"`. For a local target, set `source`
instead of `git`.

## Development

```bash
nix develop
cargo test
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for work tracking, documentation scope,
cross-repository coordination, and the canonical validation gate.

## CI

Forgejo Actions runs `.forgejo/workflows/ci.yml` for pull requests targeting
`main`, pushes to `main`, and manual dispatches. The workflow and local
development use the same repository-owned entrypoint:

```bash
bash scripts/ci/check-maki.sh
```

## License

Maki is available under the [MIT License](LICENSE).
