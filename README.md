# Maki

Maki is a line-first lightweight markup language and personal wiki runtime for
`.maki` files.

It is built around a project directory, a small `maki.toml` manifest, and plain
text notes that can be served as HTML.

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
maki serve .
maki build docs/index.maki > index.html
maki lsp
```

`maki serve <path>` walks upward from `<path>` to find `maki.toml`. If `<path>`
is the project root or inside the configured source root, it serves that source
root. Otherwise, it serves `<path>` as a plain note directory.

`maki build <file>` uses the same project discovery. Files inside the configured
source root get project-aware link resolution; other files render as standalone
HTML.

`maki lsp` starts the stdio language server for editor integration.

## Configuration

The project manifest is `maki.toml` at the project root. See
[docs/maki-toml.maki](docs/maki-toml.maki) for the full configuration
contract, including `title`, `source`, and `home`.

## Syntax

Common `.maki` building blocks:

```maki
--^ title: Example

= Heading

Paragraph with a [[note link]], [reference link], and <https://example.com>.

[reference link]: https://example.com

- [ ] Todo item
- [x] Done item

: code line
```

See [docs/maki-syntax.maki](docs/maki-syntax.maki) for the syntax source of
truth.

## Web Routes

- `/`: redirect to the configured home note.
- `/<note>`: rendered note page.
- `/<note>.maki`: raw source text.
- `/@/`: meta index.
- `/@/recents`: recently modified notes.
- `/@/diagnostics`: project diagnostics.
- `/@/dates`: date index.
- `/.maki/search`: title search.

## Development

```bash
nix develop
cargo test
```
