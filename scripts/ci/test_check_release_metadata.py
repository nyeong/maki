import importlib.util
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


sys.dont_write_bytecode = True
SCRIPT_PATH = Path(__file__).with_name("check-release-metadata.py")
SPEC = importlib.util.spec_from_file_location("check_release_metadata", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
CHECK_RELEASE_METADATA = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK_RELEASE_METADATA)

VERSION = "0.1.0"
REPOSITORY = "https://git.eska.nyeong.me/nyeong/maki"
LICENSE_TEXT = """
MIT License

Copyright (c) 2026 An Nyeong

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"""


class ReleaseMetadataTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.repository_root = Path(self.temporary_directory.name)
        self.write_valid_repository()

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def write(self, relative_path: str, contents: str) -> None:
        path = self.repository_root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(textwrap.dedent(contents).lstrip(), encoding="utf-8")

    def write_valid_repository(self) -> None:
        inherited = """
            version.workspace = true
            authors.workspace = true
            edition.workspace = true
            license.workspace = true
            repository.workspace = true
            homepage.workspace = true
            publish.workspace = true
        """
        self.write(
            "Cargo.toml",
            f"""
            [package]
            name = "maki"
            {textwrap.dedent(inherited)}
            description = "Maki"

            [workspace]
            members = ["crates/maki-core"]

            [workspace.package]
            version = "{VERSION}"
            authors = ["An Nyeong <me@annyeong.me>"]
            edition = "2024"
            license = "MIT"
            repository = "{REPOSITORY}"
            homepage = "{REPOSITORY}"
            publish = false

            [dependencies]
            maki-core = {{ version = "{VERSION}", path = "crates/maki-core" }}
            """,
        )
        self.write(
            "crates/maki-core/Cargo.toml",
            f"""
            [package]
            name = "maki-core"
            {textwrap.dedent(inherited)}
            description = "Maki core"
            """,
        )
        self.write(
            "Cargo.lock",
            f"""
            version = 4

            [[package]]
            name = "maki"
            version = "{VERSION}"

            [[package]]
            name = "maki-core"
            version = "{VERSION}"
            """,
        )
        self.write("LICENSE", LICENSE_TEXT)
        self.write(
            "flake.nix",
            f'''
            {{ self, ... }}:
            {{
              inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
              packages.default = buildRustPackage {{
                version = "{VERSION}";
                MAKI_SOURCE_REVISION = self.rev or "";
                postInstall = "touch $out/share/licenses/maki/LICENSE";
                meta = {{
                  homepage = "{REPOSITORY}";
                  license = pkgs.lib.licenses.mit;
                }};
              }};
            }}
            ''',
        )
        self.write(
            "README.md",
            f"""
            [source]({REPOSITORY})
            [release contract](RELEASES.md)
            [changelog](CHANGELOG.md)
            [MIT License](LICENSE)
            """,
        )
        self.write(
            "flake.lock",
            """
            {
              "nodes": {
                "nixpkgs": {
                  "locked": {
                    "narHash": "sha256-test",
                    "owner": "NixOS",
                    "repo": "nixpkgs",
                    "rev": "1111111111111111111111111111111111111111",
                    "type": "github"
                  },
                  "original": {
                    "owner": "NixOS",
                    "repo": "nixpkgs",
                    "type": "github"
                  }
                },
                "root": {"inputs": {"nixpkgs": "nixpkgs"}}
              },
              "root": "root",
              "version": 7
            }
            """,
        )
        self.write("CHANGELOG.md", "## [Unreleased]\n")
        self.write(
            "RELEASES.md",
            f"""
            {REPOSITORY}
            maki-vMAJOR.MINOR.PATCH
            source_revision
            """,
        )
        self.write("docs/index.maki", REPOSITORY)

    def errors(self) -> list[str]:
        return CHECK_RELEASE_METADATA.metadata_errors(self.repository_root)

    def assert_error_contains(
        self, expected: str, errors: list[str] | None = None
    ) -> None:
        actual_errors = self.errors() if errors is None else errors
        self.assertTrue(
            any(expected in error for error in actual_errors),
            f"expected an error containing {expected!r}, got {actual_errors!r}",
        )

    def replace(self, relative_path: str, old: str, new: str) -> None:
        path = self.repository_root / relative_path
        contents = path.read_text(encoding="utf-8")
        self.assertIn(old, contents)
        path.write_text(contents.replace(old, new, 1), encoding="utf-8")

    def test_accepts_public_consistent_metadata_and_relative_paths(self) -> None:
        self.assertEqual(self.errors(), [])

    def test_reports_missing_license(self) -> None:
        (self.repository_root / "LICENSE").unlink()

        self.assertIn("cannot read LICENSE", self.errors())

    def test_rejects_truncated_license(self) -> None:
        self.write("LICENSE", "MIT License\nCopyright (c) 2026 An Nyeong\n")

        self.assertIn(
            "LICENSE does not match the approved MIT license text", self.errors()
        )

    def test_rejects_non_public_git_dependency(self) -> None:
        self.replace(
            "Cargo.toml",
            "[dependencies]\n",
            '[dependencies]\nprivate = { git = "ssh://git@private.invalid/maki" }\n',
        )

        self.assert_error_contains("non-public Git source")

    def test_malformed_public_url_is_rejected_without_crashing(self) -> None:
        self.assertFalse(CHECK_RELEASE_METADATA.is_public_https_url("https://["))

    def test_rejects_floating_public_git_dependency(self) -> None:
        self.replace(
            "Cargo.toml",
            "[dependencies]\n",
            '[dependencies]\nremote = { git = "https://github.com/example/repo" }\n',
        )

        self.assert_error_contains("needs a full Git revision")

    def test_rejects_private_network_git_dependency(self) -> None:
        self.replace(
            "Cargo.toml",
            "[dependencies]\n",
            '[dependencies]\nremote = { git = "https://127.0.0.1/repo", rev = "1111111111111111111111111111111111111111" }\n',
        )

        self.assert_error_contains("non-public Git source")

    def test_rejects_absolute_dependency_path(self) -> None:
        self.replace(
            "Cargo.toml", 'path = "crates/maki-core"', 'path = "/home/user/maki-core"'
        )

        errors = self.errors()
        self.assert_error_contains("uses an absolute path", errors)
        self.assert_error_contains("does not resolve to its workspace member", errors)

    def test_rejects_file_dependency_path(self) -> None:
        self.replace(
            "Cargo.toml",
            'path = "crates/maki-core"',
            'path = "file:///home/user/maki-core"',
        )

        self.assert_error_contains("machine-local path source")

    def test_reports_internal_dependency_version_mismatch(self) -> None:
        self.replace(
            "Cargo.toml", f'version = "{VERSION}", path', 'version = "9.9.9", path'
        )

        self.assert_error_contains("must require version 0.1.0")

    def test_discovers_implicit_path_workspace_member(self) -> None:
        self.replace(
            "Cargo.toml",
            "[dependencies]\n",
            '[dependencies]\nhelper = { version = "9.9.9", path = "crates/helper" }\n',
        )
        self.write(
            "crates/helper/Cargo.toml",
            """
            [package]
            name = "helper"
            version = "9.9.9"
            edition = "2024"
            description = "Implicit member"
            """,
        )

        errors = self.errors()
        self.assert_error_contains("package helper must inherit version", errors)
        self.assert_error_contains("helper must require version 0.1.0", errors)

    def test_allows_features_on_inherited_workspace_dependency(self) -> None:
        self.replace(
            "Cargo.toml",
            "[dependencies]\n",
            '[workspace.dependencies]\nserde = "1"\n\n[dependencies]\n',
        )
        self.replace(
            "crates/maki-core/Cargo.toml",
            'description = "Maki core"',
            'description = "Maki core"\n\n'
            '[dependencies]\nserde = { workspace = true, features = ["derive"] }',
        )

        self.assertEqual(self.errors(), [])

    def test_rejects_private_workspace_and_patch_sources(self) -> None:
        self.replace(
            "Cargo.toml",
            f'maki-core = {{ version = "{VERSION}", path = "crates/maki-core" }}',
            f'maki-core = {{ version = "{VERSION}", path = "crates/maki-core" }}\n\n'
            '[workspace.dependencies]\nhelper = { path = "/home/user/helper" }\n\n'
            '[patch.crates-io]\npatched = { path = "../private" }',
        )

        errors = self.errors()
        self.assert_error_contains("helper uses an absolute path", errors)
        self.assert_error_contains("patched escapes the repository", errors)

    def test_rejects_private_cargo_source_replacement(self) -> None:
        self.write(
            ".cargo/config.toml",
            """
            [source.crates-io]
            replace-with = "local"

            [source.local]
            directory = "/home/user/vendor"
            """,
        )

        self.assert_error_contains("is machine-local")

    def test_rejects_private_and_floating_cargo_git_sources(self) -> None:
        self.write(
            ".cargo/config.toml",
            """
            [source.private]
            git = "ssh://private.invalid/repository"
            rev = "1111111111111111111111111111111111111111"

            [source.floating]
            git = "https://github.com/example/repository"
            """,
        )

        errors = self.errors()
        self.assert_error_contains("non-public Git source", errors)
        self.assert_error_contains("needs a full Git revision", errors)

    def test_reports_member_that_does_not_inherit_version(self) -> None:
        self.replace(
            "crates/maki-core/Cargo.toml",
            "version.workspace = true",
            'version = "0.1.0"',
        )

        self.assert_error_contains("must inherit version")

    def test_rejects_absolute_workspace_member(self) -> None:
        self.replace(
            "Cargo.toml",
            'members = ["crates/maki-core"]',
            'members = ["/home/user/maki-core"]',
        )

        errors = self.errors()
        self.assertTrue(
            any("workspace member" in error and "absolute" in error for error in errors),
            f"expected an absolute workspace member error, got {errors!r}",
        )

    def test_reports_lock_and_flake_version_mismatches(self) -> None:
        self.replace("Cargo.lock", 'version = "0.1.0"', 'version = "0.2.0"')
        self.replace("flake.nix", 'version = "0.1.0"', 'version = "0.2.0"')

        errors = self.errors()
        self.assert_error_contains("Cargo.lock package maki", errors)
        self.assertIn("flake package version must be 0.1.0", errors)

    def test_rejects_unapproved_flake_lock_source(self) -> None:
        self.replace(
            "flake.lock",
            '"owner": "NixOS"',
            '"owner": "private-owner"',
        )

        self.assert_error_contains("not an approved public source")

    def test_rejects_private_lock_source(self) -> None:
        self.replace(
            "Cargo.lock",
            'name = "maki-core"\nversion = "0.1.0"',
            'name = "maki-core"\nversion = "0.1.0"\nsource = "git+ssh://private.invalid/maki-core"',
        )

        self.assert_error_contains("uses a non-public source")

    def test_rejects_obsolete_repository_link(self) -> None:
        self.write(
            "docs/index.maki",
            f"{REPOSITORY}\nhttps://github.com/nyeong/maki\n",
        )

        self.assertIn(
            "docs/index.maki contains the obsolete repository URL", self.errors()
        )

    def test_release_mode_requires_dated_version_and_tag(self) -> None:
        errors = CHECK_RELEASE_METADATA.metadata_errors(
            self.repository_root, release=True
        )
        self.assert_error_contains("dated 0.1.0", errors)
        self.assert_error_contains("maki-v0.1.0", errors)

        self.write(
            "CHANGELOG.md",
            f"""
            ## [Unreleased]

            ## [0.1.0] - 2026-08-31

            [0.1.0]: {REPOSITORY}/releases/tag/maki-v0.1.0
            """,
        )
        self.assertEqual(
            CHECK_RELEASE_METADATA.metadata_errors(
                self.repository_root, release=True
            ),
            [],
        )

    def test_release_mode_rejects_invalid_calendar_date(self) -> None:
        self.write(
            "CHANGELOG.md",
            f"""
            ## [Unreleased]

            ## [0.1.0] - 2026-99-99

            [0.1.0]: {REPOSITORY}/releases/tag/maki-v0.1.0
            """,
        )

        self.assert_error_contains(
            "invalid 0.1.0 release date",
            CHECK_RELEASE_METADATA.metadata_errors(
                self.repository_root, release=True
            ),
        )

    def test_release_checkout_must_be_a_clean_commit(self) -> None:
        subprocess.run(
            ["git", "init", "--quiet"], cwd=self.repository_root, check=True
        )
        subprocess.run(
            ["git", "add", "--all"], cwd=self.repository_root, check=True
        )
        subprocess.run(
            [
                "git",
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
            cwd=self.repository_root,
            check=True,
        )
        self.assertEqual(
            CHECK_RELEASE_METADATA.release_repository_errors(self.repository_root),
            [],
        )

        with (self.repository_root / "README.md").open("a", encoding="utf-8") as file:
            file.write("dirty\n")
        self.assertIn(
            "release checkout has uncommitted changes",
            CHECK_RELEASE_METADATA.release_repository_errors(self.repository_root),
        )


if __name__ == "__main__":
    unittest.main()
