# Maki

Maki is a simple, readable, and extensible markup language.

The canonical source is maintained in a private [Forgejo repository](https://git.eska.nyeong.me/nyeong/maki).
A public mirror is available at [nyeong/maki](https://github.com/nyeong/maki).
The CLI and LSP share one release version; see the [release contract](RELEASES.md) and
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
maki build docs/index.maki --check-external-links > index.html
maki check .
maki check docs/index.maki --format json
maki fmt .
maki fmt --check
maki fmt - < draft.maki > formatted.maki
maki lsp
maki serve --git https://example.invalid/maki.git --branch main --state-dir /var/lib/maki/docs
maki serve . --metrics 127.0.0.1:4041
```

`maki serve <path>` walks upward from `<path>` to find `maki.toml`. If `<path>`
is the project root or inside the configured source root, it serves that source
root. Otherwise, it serves `<path>` as a plain note directory.

`maki build <file>` uses the same project discovery. Files inside the configured
source root get project-aware link resolution; other files render as standalone
HTML. Builds are offline by default: parser and project diagnostics never make a
network request. Pass `--check-external-links` to explicitly check authored
HTTP(S) links and include any failures in the warnings written to stderr. This
opt-in is a CLI policy and cannot be enabled by `maki.toml`.

`maki fmt [path]` normalizes conservative structural whitespace in one `.maki`
file or every note in a project, without reflowing paragraphs or changing raw
container bodies. The path defaults to the current directory. A project path
uses its configured source root; a directory outside that source is handled as
a standalone note directory. Pass `--check` to report files that would change
without writing them. A dirty check exits with status 1. Use `-` to read one
document from stdin and write the formatted document to stdout.

`maki check [path]` runs the parser and semantic project diagnostics without
writing source files, contacting the network, or opening an editor. The path
defaults to the current directory. A directory in a discovered project checks
the configured source root; a standalone directory checks all of its notes.
A file in a project is analyzed with the full project context, while only
findings owned by that file are reported. A file outside a project receives
parser and file-local validation.

Text output uses `path:line:column: severity[code]: message`. Pass
`--format json` for the versioned machine-readable report; JSON byte ranges are
zero-based, half-open UTF-8 ranges, while its line and column values are
one-based Unicode-scalar positions. Project paths are source-root-relative and
use `/` separators. A successful clean check exits 0, findings exit 1, and
usage or operational failures exit 2. If only some sources cannot be read,
readable findings remain in the report, JSON sets `complete` to `false`, and
the command exits 2. Reports go to stdout and operational details to stderr.

Formatting and validation have separate jobs: the LSP reports the same Core
diagnostics interactively for open editor buffers, `maki fmt --check` only
detects non-canonical syntax formatting, and `maki check` validates the
on-disk snapshot as a batch or CI gate. Neither formatting nor checking runs an
evaluator or updates generated results.

Core also defines an IO-free ownership, freshness, effect, and source-edit
plan contract for materializer implementations. In this revision those values
are not wired into parsing, project diagnostics, the CLI, or the LSP: there is no
`maki update` command or materialization code action, and reserved properties
remain ordinary preserved metadata. See the
[materialization contract](docs/materialization.maki) for the exact current
boundary. Declaring an effect in source never grants `filesystem-read`,
`process`, `network`, or `clock/random/secrets` capabilities.

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
- [Materialization contract and current boundaries](docs/materialization.maki)
- [Documentation index](docs/index.maki)

## Web Routes

- `/`: redirect to the configured home note.
- `/<note>`: rendered note page.
- `/<note>/`: direct subdocument index for the note, including an empty state.
- `/<note>.maki`: raw source text for local serving; always 404 for Git serving.
- `/@/`: meta index.
- `/@/recents`: recently modified notes.
- `/@/sitemap`: human-readable sitemap.
- `/@/diagnostics`: project diagnostics.
- `/@/dates`: date index.
- `/.maki/search`: note, heading, explicit ID, and file search.
- `/.maki/project-index.json`: versioned project analysis.
- `/.maki/search-index.json`: search entries.

## Publishing

Local `maki serve <path>` is a private authoring runtime and serves every
readable project document. `maki serve --git <url>` is a public runtime: each
public document must opt in independently with a root document property whose
value is the exact lowercase token `all`:

```maki
--^ publish: all
```

Missing, unknown, block-level, nested, or unreadable declarations are private,
and publishing a parent document does not publish its children. The same public
document set governs rendered pages, hierarchy navigation, the home redirect,
search, sitemaps, recents, dates and backlinks, diagnostics, project JSON,
caches, and public note metrics. If the configured home is private, `/` returns
404 without a `Location` header.

Public serving returns 404 for every raw `.maki` source route, including public
documents. Internal note links whose targets are private, missing, or ambiguous
all render as the same target-free `[데이터 말소]` text.

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
