#!/usr/bin/env python3
import argparse
import json
import re
import sys
import tomllib
from pathlib import Path
from typing import Any


REVISION = re.compile(r"^[0-9a-f]{40}$")


def nested_value(document: dict[str, Any], *path: str) -> Any:
    value: Any = document
    for component in path:
        if not isinstance(value, dict) or component not in value:
            raise KeyError(".".join(path))
        value = value[component]
    return value


def flake_revision(source: str, input_name: str) -> str | None:
    if input_name == "maki":
        match = re.search(
            r'^\s*maki\.url\s*=\s*"[^"\n]*[?&]rev=([0-9a-fA-F]{40})(?=&|")[^"\n]*";',
            source,
            re.MULTILINE,
        )
    else:
        block = re.search(
            rf"^\s*{re.escape(input_name)}\s*=\s*\{{(?P<body>.*?)^\s*\}};",
            source,
            re.MULTILINE | re.DOTALL,
        )
        match = (
            re.search(
                r'^\s*url\s*=\s*"[^"\n]*[?&]rev=([0-9a-fA-F]{40})(?=&|")[^"\n]*";',
                block.group("body"),
                re.MULTILINE,
            )
            if block
            else None
        )
    return match.group(1).lower() if match else None


def lock_input_node(lock: dict[str, Any], input_name: str) -> dict[str, Any]:
    root_name = nested_value(lock, "root")
    if not isinstance(root_name, str):
        raise KeyError("root")
    root = nested_value(lock, "nodes", root_name)
    node_name = nested_value(root, "inputs", input_name)
    if not isinstance(node_name, str):
        raise KeyError(f"{root_name}.inputs.{input_name}")
    node = nested_value(lock, "nodes", node_name)
    if not isinstance(node, dict):
        raise KeyError(f"nodes.{node_name}")
    return node


def metadata_errors(
    extension_dir: Path, expected_maki: str, expected_grammar: str
) -> list[str]:
    try:
        with (extension_dir / "extension.toml").open("rb") as file:
            manifest = tomllib.load(file)
        flake_source = (extension_dir / "flake.nix").read_text(encoding="utf-8")
        with (extension_dir / "flake.lock").open(encoding="utf-8") as file:
            lock = json.load(file)
    except OSError:
        return ["cannot read maki-zed compatibility metadata"]
    except (json.JSONDecodeError, tomllib.TOMLDecodeError):
        return ["maki-zed compatibility metadata is not valid JSON/TOML"]

    try:
        maki_node = lock_input_node(lock, "maki")
    except KeyError:
        maki_node = {}
    try:
        grammar_node = lock_input_node(lock, "tree-sitter-maki")
    except KeyError:
        grammar_node = {}

    flake_maki = flake_revision(flake_source, "maki")
    flake_grammar = flake_revision(flake_source, "tree-sitter-maki")

    checks = (
        (
            "extension.toml grammar revision",
            ("grammars", "maki", "rev"),
            manifest,
            expected_grammar,
        ),
        (
            "flake.nix Maki declared revision",
            ("rev",),
            {"rev": flake_maki} if flake_maki else {},
            expected_maki,
        ),
        (
            "flake.nix grammar declared revision",
            ("rev",),
            {"rev": flake_grammar} if flake_grammar else {},
            expected_grammar,
        ),
        (
            "flake.lock Maki locked revision",
            ("locked", "rev"),
            maki_node,
            expected_maki,
        ),
        (
            "flake.lock Maki declared revision",
            ("original", "rev"),
            maki_node,
            expected_maki,
        ),
        (
            "flake.lock grammar locked revision",
            ("locked", "rev"),
            grammar_node,
            expected_grammar,
        ),
        (
            "flake.lock grammar declared revision",
            ("original", "rev"),
            grammar_node,
            expected_grammar,
        ),
    )

    errors = []
    for label, path, document, expected in checks:
        try:
            actual = nested_value(document, *path)
        except KeyError:
            errors.append(f"{label} is missing")
            continue
        if actual != expected:
            displayed = (
                actual
                if isinstance(actual, str) and REVISION.fullmatch(actual)
                else "<invalid revision>"
            )
            errors.append(f"{label} mismatch: expected {expected}, found {displayed}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("extension_dir", type=Path)
    parser.add_argument("maki_revision")
    parser.add_argument("grammar_revision")
    args = parser.parse_args()

    errors = metadata_errors(
        args.extension_dir, args.maki_revision, args.grammar_revision
    )
    if errors:
        for error in errors:
            print(f"check-stack: {error}", file=sys.stderr)
        return 1

    print("maki-zed manifest and flake lock match the selected revisions.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
