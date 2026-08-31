#!/usr/bin/env python3
import argparse
import hashlib
import ipaddress
import json
import re
import subprocess
import sys
import tomllib
from datetime import date
from pathlib import Path
from typing import Any, Iterator
from urllib.parse import urlsplit


REPOSITORY_URL = "https://git.eska.nyeong.me/nyeong/maki"
LICENSE_ID = "MIT"
LICENSE_SHA256 = "50842177efebc721b7faf0a1a8e3527d2708e0f0d669dd17b5720de29f57fc32"
INHERITED_FIELDS = (
    "version",
    "authors",
    "edition",
    "license",
    "repository",
    "homepage",
    "publish",
)
SEMANTIC_VERSION = re.compile(
    r"^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$"
)
WINDOWS_ABSOLUTE_PATH = re.compile(r"^[A-Za-z]:[\\/]")
NIX_VERSION_ASSIGNMENT = re.compile(
    r'^\s*version\s*=\s*"([^"]+)";', re.MULTILINE
)
FULL_REVISION = re.compile(r"^[0-9a-fA-F]{40}$")
PRIVATE_HOST_SUFFIXES = (
    ".example",
    ".home.arpa",
    ".internal",
    ".invalid",
    ".local",
    ".localhost",
    ".test",
)
PUBLIC_FLAKE_REPOSITORIES = {
    ("NixOS", "flake-compat"),
    ("NixOS", "nixpkgs"),
    ("cachix", "git-hooks.nix"),
    ("hercules-ci", "gitignore.nix"),
}


def load_toml(path: Path, errors: list[str]) -> dict[str, Any] | None:
    try:
        with path.open("rb") as file:
            document = tomllib.load(file)
    except OSError:
        errors.append(f"cannot read {path.name}")
        return None
    except tomllib.TOMLDecodeError:
        errors.append(f"{path.name} is not valid TOML")
        return None
    return document


def read_text(path: Path, errors: list[str]) -> str | None:
    try:
        return path.read_text(encoding="utf-8")
    except OSError:
        errors.append(f"cannot read {path.name}")
        return None


def load_json(path: Path, errors: list[str]) -> dict[str, Any] | None:
    try:
        with path.open(encoding="utf-8") as file:
            document = json.load(file)
    except OSError:
        errors.append(f"cannot read {path.name}")
        return None
    except json.JSONDecodeError:
        errors.append(f"{path.name} is not valid JSON")
        return None
    if not isinstance(document, dict):
        errors.append(f"{path.name} must contain a JSON object")
        return None
    return document


def is_inherited(value: Any) -> bool:
    return value == {"workspace": True}


def is_public_https_url(
    value: str, *, allow_query: bool = False, allow_fragment: bool = False
) -> bool:
    try:
        parsed = urlsplit(value)
    except ValueError:
        return False
    hostname = parsed.hostname
    if (
        parsed.scheme
        not in {"https", "git+https", "registry+https", "sparse+https"}
        or hostname is None
        or parsed.username is not None
        or parsed.password is not None
        or (parsed.query and not allow_query)
        or (parsed.fragment and not allow_fragment)
    ):
        return False

    normalized_hostname = hostname.rstrip(".").lower()
    if any(
        normalized_hostname == suffix[1:] or normalized_hostname.endswith(suffix)
        for suffix in PRIVATE_HOST_SUFFIXES
    ):
        return False
    try:
        address = ipaddress.ip_address(normalized_hostname)
    except ValueError:
        return "." in normalized_hostname
    return address.is_global


def has_immutable_git_revision(specification: dict[str, Any]) -> bool:
    revision = specification.get("rev")
    return isinstance(revision, str) and FULL_REVISION.fullmatch(revision) is not None


def check_git_source(
    specification: dict[str, Any], label: str, errors: list[str]
) -> None:
    git_source = specification.get("git")
    if git_source is None:
        return
    if not isinstance(git_source, str) or not is_public_https_url(git_source):
        errors.append(f"{label} uses a non-public Git source")
    if not has_immutable_git_revision(specification):
        errors.append(f"{label} needs a full Git revision")


def is_public_locked_source(source: str) -> bool:
    parsed = urlsplit(source)
    if parsed.scheme == "git+https":
        return (
            is_public_https_url(source, allow_query=True, allow_fragment=True)
            and FULL_REVISION.fullmatch(parsed.fragment) is not None
        )
    return is_public_https_url(source)


def source_tables(manifest: dict[str, Any]) -> Iterator[tuple[dict[str, Any], bool]]:
    for name in ("dependencies", "dev-dependencies", "build-dependencies"):
        table = manifest.get(name)
        if isinstance(table, dict):
            yield table, True

    workspace = manifest.get("workspace")
    if isinstance(workspace, dict):
        table = workspace.get("dependencies")
        if isinstance(table, dict):
            yield table, True

    targets = manifest.get("target")
    if isinstance(targets, dict):
        for target in targets.values():
            if not isinstance(target, dict):
                continue
            for name in ("dependencies", "dev-dependencies", "build-dependencies"):
                table = target.get(name)
                if isinstance(table, dict):
                    yield table, True

    patches = manifest.get("patch")
    if isinstance(patches, dict):
        for table in patches.values():
            if isinstance(table, dict):
                yield table, False

    replacements = manifest.get("replace")
    if isinstance(replacements, dict):
        yield replacements, False


def member_manifest_paths(
    repository_root: Path, workspace: dict[str, Any], errors: list[str]
) -> list[Path]:
    members = workspace.get("members")
    if not isinstance(members, list) or not all(
        isinstance(member, str) for member in members
    ):
        errors.append("Cargo workspace members are missing or invalid")
        return []

    manifests: list[Path] = []
    for member in members:
        if Path(member).is_absolute() or WINDOWS_ABSOLUTE_PATH.match(member):
            errors.append(f"workspace member {member} uses an absolute path")
            continue
        try:
            matches = sorted(repository_root.glob(f"{member}/Cargo.toml"))
        except (NotImplementedError, ValueError):
            errors.append(f"workspace member {member} has an invalid path")
            continue
        if not matches:
            errors.append(f"workspace member {member} has no Cargo.toml")
        for manifest_path in matches:
            if not path_is_within(manifest_path.resolve(), repository_root):
                errors.append(f"workspace member {member} escapes the repository")
                continue
            manifests.append(manifest_path)
    return manifests


def path_is_within(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
    except ValueError:
        return False
    return True


def discover_path_workspace_members(
    repository_root: Path,
    workspace: dict[str, Any],
    manifests: dict[Path, dict[str, Any]],
    errors: list[str],
) -> None:
    exclude = workspace.get("exclude", [])
    exclude_patterns = (
        [pattern for pattern in exclude if isinstance(pattern, str)]
        if isinstance(exclude, list)
        else []
    )
    pending = list(manifests.items())
    while pending:
        manifest_path, manifest = pending.pop()
        for table, is_dependency_table in source_tables(manifest):
            if not is_dependency_table:
                continue
            for specification in table.values():
                if not isinstance(specification, dict):
                    continue
                path_source = specification.get("path")
                if not isinstance(path_source, str):
                    continue
                package_dir = (manifest_path.parent / path_source).resolve()
                if not path_is_within(package_dir, repository_root):
                    continue
                relative_dir = package_dir.relative_to(repository_root)
                if any(relative_dir.match(pattern) for pattern in exclude_patterns):
                    continue
                dependency_manifest = package_dir / "Cargo.toml"
                if dependency_manifest in manifests or not dependency_manifest.is_file():
                    continue
                dependency = load_toml(dependency_manifest, errors)
                if dependency is not None:
                    manifests[dependency_manifest] = dependency
                    pending.append((dependency_manifest, dependency))


def check_dependency_sources(
    repository_root: Path,
    manifest_path: Path,
    manifest: dict[str, Any],
    workspace_packages: dict[str, Path],
    release_version: str,
    errors: list[str],
) -> None:
    for table, is_dependency_table in source_tables(manifest):
        for dependency_name, specification in table.items():
            if isinstance(specification, str):
                if is_dependency_table and dependency_name in workspace_packages:
                    errors.append(
                        f"{manifest_path}: internal dependency {dependency_name} "
                        "must use its versioned workspace path"
                    )
                continue
            if not isinstance(specification, dict):
                continue
            if specification.get("workspace") is True:
                unsupported_fields = set(specification) - {
                    "default-features",
                    "features",
                    "optional",
                    "workspace",
                }
                if unsupported_fields:
                    errors.append(
                        f"{manifest_path}: dependency {dependency_name} has invalid "
                        "workspace source fields"
                    )
                continue

            package_name = specification.get("package", dependency_name)
            check_git_source(
                specification,
                f"{manifest_path}: dependency {dependency_name}",
                errors,
            )

            path_source = specification.get("path")
            resolved_path: Path | None = None
            if isinstance(path_source, str):
                if path_source.startswith(("file:", "path:", "ssh:")):
                    errors.append(
                        f"{manifest_path}: dependency {dependency_name} uses a "
                        "machine-local path source"
                    )
                elif Path(path_source).is_absolute() or WINDOWS_ABSOLUTE_PATH.match(
                    path_source
                ):
                    errors.append(
                        f"{manifest_path}: dependency {dependency_name} uses an "
                        "absolute path"
                    )
                else:
                    resolved_path = (manifest_path.parent / path_source).resolve()
                    if not path_is_within(resolved_path, repository_root):
                        errors.append(
                            f"{manifest_path}: dependency {dependency_name} "
                            "escapes the repository"
                        )

            if (
                not is_dependency_table
                or not isinstance(package_name, str)
                or package_name not in workspace_packages
            ):
                continue
            if specification.get("version") != release_version:
                errors.append(
                    f"{manifest_path}: internal dependency {dependency_name} "
                    f"must require version {release_version}"
                )
            expected_path = workspace_packages[package_name].parent.resolve()
            if resolved_path != expected_path:
                errors.append(
                    f"{manifest_path}: internal dependency {dependency_name} "
                    "does not resolve to its workspace member"
                )


def check_cargo_config(repository_root: Path, errors: list[str]) -> None:
    config_paths = [
        path
        for path in (
            repository_root / ".cargo/config.toml",
            repository_root / ".cargo/config",
        )
        if path.exists()
    ]
    if len(config_paths) > 1:
        errors.append("both .cargo/config and .cargo/config.toml are present")
    for config_path in config_paths:
        if not path_is_within(config_path.resolve(), repository_root):
            errors.append(f"{config_path}: Cargo config escapes the repository")
            continue
        config = load_toml(config_path, errors)
        if config is None:
            continue
        if config.get("paths"):
            errors.append(f"{config_path}: Cargo path overrides are not public sources")

        sources = config.get("source")
        if isinstance(sources, dict):
            for source_name, source in sources.items():
                if not isinstance(source, dict):
                    continue
                if source.get("directory") is not None or source.get(
                    "local-registry"
                ) is not None:
                    errors.append(
                        f"{config_path}: Cargo source {source_name} is machine-local"
                    )
                registry = source.get("registry")
                if isinstance(registry, str) and not is_public_https_url(registry):
                    errors.append(
                        f"{config_path}: Cargo source {source_name} is not public HTTPS"
                    )
                check_git_source(
                    source, f"{config_path}: Cargo source {source_name}", errors
                )

        registries = config.get("registries")
        if isinstance(registries, dict):
            for registry_name, registry in registries.items():
                if not isinstance(registry, dict):
                    continue
                index = registry.get("index")
                if isinstance(index, str) and not is_public_https_url(index):
                    errors.append(
                        f"{config_path}: Cargo registry {registry_name} is not public HTTPS"
                    )


def check_flake_lock(repository_root: Path, errors: list[str]) -> None:
    lock = load_json(repository_root / "flake.lock", errors)
    if lock is None:
        return
    nodes = lock.get("nodes")
    root_name = lock.get("root")
    if not isinstance(nodes, dict) or not isinstance(root_name, str):
        errors.append("flake.lock node metadata is missing")
        return

    for node_name, node in nodes.items():
        if node_name == root_name:
            continue
        if not isinstance(node, dict):
            errors.append(f"flake.lock node {node_name} is invalid")
            continue
        locked = node.get("locked")
        original = node.get("original")
        if not isinstance(locked, dict) or not isinstance(original, dict):
            errors.append(f"flake.lock node {node_name} is not immutable")
            continue

        locked_repository = (locked.get("owner"), locked.get("repo"))
        original_repository = (original.get("owner"), original.get("repo"))
        if (
            locked.get("type") != "github"
            or original.get("type") != "github"
            or locked_repository not in PUBLIC_FLAKE_REPOSITORIES
            or original_repository != locked_repository
        ):
            errors.append(
                f"flake.lock node {node_name} is not an approved public source"
            )
        revision = locked.get("rev")
        if not isinstance(revision, str) or FULL_REVISION.fullmatch(revision) is None:
            errors.append(f"flake.lock node {node_name} lacks an immutable revision")
        nar_hash = locked.get("narHash")
        if not isinstance(nar_hash, str) or not nar_hash.startswith("sha256-"):
            errors.append(f"flake.lock node {node_name} lacks a content hash")


def release_repository_errors(repository_root: Path) -> list[str]:
    errors: list[str] = []
    revision = subprocess.run(
        ["git", "-C", str(repository_root), "rev-parse", "--verify", "HEAD^{commit}"],
        check=False,
        capture_output=True,
        text=True,
    )
    if revision.returncode != 0 or FULL_REVISION.fullmatch(revision.stdout.strip()) is None:
        errors.append("release checkout does not have a valid Git HEAD")

    status = subprocess.run(
        [
            "git",
            "-C",
            str(repository_root),
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    if status.returncode != 0:
        errors.append("cannot inspect release checkout status")
    elif status.stdout:
        errors.append("release checkout has uncommitted changes")
    return errors


def metadata_errors(repository_root: Path, *, release: bool = False) -> list[str]:
    repository_root = repository_root.resolve()
    errors: list[str] = []
    root_manifest_path = repository_root / "Cargo.toml"
    root_manifest = load_toml(root_manifest_path, errors)
    if root_manifest is None:
        return errors

    workspace = root_manifest.get("workspace")
    if not isinstance(workspace, dict):
        return [*errors, "Cargo workspace metadata is missing"]
    workspace_package = workspace.get("package")
    if not isinstance(workspace_package, dict):
        return [*errors, "workspace.package metadata is missing"]

    release_version = workspace_package.get("version")
    if not isinstance(release_version, str) or not SEMANTIC_VERSION.fullmatch(
        release_version
    ):
        errors.append("workspace package version is missing or invalid")
        release_version = "<invalid>"

    expected_workspace_metadata: dict[str, Any] = {
        "authors": ["An Nyeong <me@annyeong.me>"],
        "edition": "2024",
        "license": LICENSE_ID,
        "repository": REPOSITORY_URL,
        "homepage": REPOSITORY_URL,
        "publish": False,
    }
    for field, expected in expected_workspace_metadata.items():
        if workspace_package.get(field) != expected:
            errors.append(f"workspace package {field} is missing or inconsistent")

    manifest_paths = [
        root_manifest_path,
        *member_manifest_paths(repository_root, workspace, errors),
    ]
    manifests: dict[Path, dict[str, Any]] = {root_manifest_path: root_manifest}
    for manifest_path in manifest_paths[1:]:
        manifest = load_toml(manifest_path, errors)
        if manifest is not None:
            manifests[manifest_path] = manifest
    discover_path_workspace_members(repository_root, workspace, manifests, errors)

    workspace_packages: dict[str, Path] = {}
    for manifest_path, manifest in manifests.items():
        package = manifest.get("package")
        if not isinstance(package, dict):
            errors.append(f"{manifest_path}: package metadata is missing")
            continue
        package_name = package.get("name")
        if not isinstance(package_name, str):
            errors.append(f"{manifest_path}: package name is missing")
            continue
        if package_name in workspace_packages:
            errors.append(f"duplicate workspace package {package_name}")
        workspace_packages[package_name] = manifest_path
        for field in INHERITED_FIELDS:
            if not is_inherited(package.get(field)):
                errors.append(
                    f"{manifest_path}: package {package_name} must inherit {field}"
                )
        if not isinstance(package.get("description"), str):
            errors.append(f"{manifest_path}: package {package_name} needs a description")

    for manifest_path, manifest in manifests.items():
        check_dependency_sources(
            repository_root,
            manifest_path,
            manifest,
            workspace_packages,
            release_version,
            errors,
        )
    check_cargo_config(repository_root, errors)

    lock = load_toml(repository_root / "Cargo.lock", errors)
    if lock is not None:
        locked_packages = lock.get("package")
        if not isinstance(locked_packages, list):
            errors.append("Cargo.lock package metadata is missing")
        else:
            for package in locked_packages:
                if not isinstance(package, dict):
                    continue
                source = package.get("source")
                if isinstance(source, str) and not is_public_locked_source(source):
                    errors.append(
                        f"Cargo.lock package {package.get('name', '<unknown>')} "
                        "uses a non-public source"
                    )
            for package_name in workspace_packages:
                versions = [
                    package.get("version")
                    for package in locked_packages
                    if isinstance(package, dict)
                    and package.get("name") == package_name
                ]
                if versions != [release_version]:
                    errors.append(
                        f"Cargo.lock package {package_name} must have version "
                        f"{release_version}"
                    )

    license_text = read_text(repository_root / "LICENSE", errors)
    if license_text is not None and hashlib.sha256(
        license_text.encode("utf-8")
    ).hexdigest() != LICENSE_SHA256:
        errors.append("LICENSE does not match the approved MIT license text")

    flake_text = read_text(repository_root / "flake.nix", errors)
    if flake_text is not None:
        flake_versions = NIX_VERSION_ASSIGNMENT.findall(flake_text)
        if flake_versions != [release_version]:
            errors.append(f"flake package version must be {release_version}")
        required_flake_text = (
            f'homepage = "{REPOSITORY_URL}";',
            "license = pkgs.lib.licenses.mit;",
            'MAKI_SOURCE_REVISION = self.rev or "";',
            "$out/share/licenses/maki/LICENSE",
        )
        for expected in required_flake_text:
            if expected not in flake_text:
                errors.append(f"flake.nix is missing release metadata: {expected}")
    check_flake_lock(repository_root, errors)

    required_document_text = {
        "README.md": (
            REPOSITORY_URL,
            "[release contract](RELEASES.md)",
            "[changelog](CHANGELOG.md)",
            "[MIT License](LICENSE)",
        ),
        "CHANGELOG.md": ("## [Unreleased]",),
        "RELEASES.md": (
            REPOSITORY_URL,
            "maki-vMAJOR.MINOR.PATCH",
            "source_revision",
        ),
        "docs/index.maki": (REPOSITORY_URL,),
    }
    documents: dict[str, str] = {}
    for relative_path, required_values in required_document_text.items():
        contents = read_text(repository_root / relative_path, errors)
        if contents is None:
            continue
        documents[relative_path] = contents
        for expected in required_values:
            if expected not in contents:
                errors.append(f"{relative_path} is missing {expected}")
        if "github.com/nyeong/maki" in contents:
            errors.append(f"{relative_path} contains the obsolete repository URL")

    if release and release_version != "<invalid>":
        changelog = documents.get("CHANGELOG.md", "")
        version_heading = re.compile(
            rf"^## \[{re.escape(release_version)}\] - "
            r"(?P<date>[0-9]{4}-[0-9]{2}-[0-9]{2})$",
            re.MULTILINE,
        )
        heading = version_heading.search(changelog)
        if heading is None:
            errors.append(
                f"CHANGELOG.md needs a dated {release_version} release section"
            )
        else:
            try:
                date.fromisoformat(heading.group("date"))
            except ValueError:
                errors.append(
                    f"CHANGELOG.md has an invalid {release_version} release date"
                )
        if f"maki-v{release_version}" not in changelog:
            errors.append(
                f"CHANGELOG.md needs the maki-v{release_version} source tag"
            )

    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--release",
        action="store_true",
        help="also require a dated changelog entry for the package version",
    )
    parser.add_argument("repository_root", nargs="?", type=Path, default=Path.cwd())
    args = parser.parse_args()

    repository_root = args.repository_root.resolve()
    errors = metadata_errors(repository_root, release=args.release)
    if args.release:
        errors.extend(release_repository_errors(repository_root))
    if errors:
        for error in errors:
            print(f"release metadata: {error}", file=sys.stderr)
        return 1

    print("Release metadata is public and internally consistent.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
