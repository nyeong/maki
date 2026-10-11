<!--
README.md에는 아래의 내용만 기록한다.
1. 이게 뭔지
2. 예시
3. A를 위해서는 뭘 읽어야하는지
그 외에는 `docs/` 등 다른 곳에 기록하고 링크한다.
-->
# Maki

Maki is a simple, human-readable, and easy-to-parse markup language.

This is a personal project under heavy development. Syntax changes frequently; use with caution.

The canonical source is maintained in a private [Forgejo repository](https://git.eska.nyeong.me/nyeong/maki).
A public mirror is available at [nyeong/maki](https://github.com/nyeong/maki).

## Quick Start

In a new directory, create a file named `index.maki`:

```maki
--^ title: Title

This is a simple Maki document.
```

Render it as HTML or serve its directory locally:

```bash
maki build index.maki > index.html
maki serve .
# or use nix
nix run github:nyeong/maki -- serve .
```

See the [documentation](./docs/index.maki) for more details (Korean only).

## Goals and non-goals

Goals:

- Define the Maki syntax.
- Interpret Maki files.
- Provide a simple HTML server.
- Provide editor extensions in separate repositories.

Non-goals:

- A built-in plugin system.
- Replacing Typst.
- A high-performance HTML server.

See the [doc](./docs/about-maki.maki) for more details (Korean only).

## Developments

This project uses nix. All dependencies should be written in [flake.nix](./flake.nix). Read it to see the dependencies.

See the [release contract](RELEASES.md) for release requirements, the [changelog](CHANGELOG.md) for changes, and the [MIT License](LICENSE) for licensing terms.
