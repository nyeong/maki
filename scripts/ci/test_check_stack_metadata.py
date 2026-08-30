import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


sys.dont_write_bytecode = True
SCRIPT_PATH = Path(__file__).with_name("check-stack-metadata.py")
SPEC = importlib.util.spec_from_file_location("check_stack_metadata", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
CHECK_STACK_METADATA = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK_STACK_METADATA)

MAKI_REVISION = "1" * 40
GRAMMAR_REVISION = "2" * 40


class StackMetadataTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.extension_dir = Path(self.temporary_directory.name)

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def metadata_errors(self) -> list[str]:
        return CHECK_STACK_METADATA.metadata_errors(
            self.extension_dir, MAKI_REVISION, GRAMMAR_REVISION
        )

    def write_metadata(
        self,
        *,
        manifest_grammar: str = GRAMMAR_REVISION,
        flake_maki: str = MAKI_REVISION,
        flake_grammar: str = GRAMMAR_REVISION,
        locked_maki: str = MAKI_REVISION,
        declared_maki: str = MAKI_REVISION,
        locked_grammar: str = GRAMMAR_REVISION,
        declared_grammar: str = GRAMMAR_REVISION,
        maki_node_name: str = "maki-node",
        grammar_node_name: str = "grammar-node",
    ) -> None:
        (self.extension_dir / "extension.toml").write_text(
            "[grammars.maki]\n" f'rev = "{manifest_grammar}"\n',
            encoding="utf-8",
        )
        (self.extension_dir / "flake.nix").write_text(
            """{
  inputs = {
    maki.url = "git+https://example.invalid/maki.git?rev=%s";
    tree-sitter-maki = {
      url = "git+https://example.invalid/tree-sitter-maki.git?rev=%s";
      flake = false;
    };
  };
}
"""
            % (flake_maki, flake_grammar),
            encoding="utf-8",
        )
        lock = {
            "root": "root",
            "nodes": {
                "root": {
                    "inputs": {
                        "maki": maki_node_name,
                        "tree-sitter-maki": grammar_node_name,
                    }
                },
                maki_node_name: {
                    "locked": {"rev": locked_maki},
                    "original": {"rev": declared_maki},
                },
                grammar_node_name: {
                    "locked": {"rev": locked_grammar},
                    "original": {"rev": declared_grammar},
                },
            }
        }
        (self.extension_dir / "flake.lock").write_text(
            json.dumps(lock), encoding="utf-8"
        )

    def test_accepts_matching_manifest_and_lock(self) -> None:
        self.write_metadata()

        errors = self.metadata_errors()

        self.assertEqual(errors, [])

    def test_reports_each_mismatched_pin(self) -> None:
        unexpected = "3" * 40
        self.write_metadata(
            manifest_grammar=unexpected,
            flake_maki=unexpected,
            flake_grammar=unexpected,
            locked_maki=unexpected,
            declared_maki=unexpected,
            locked_grammar=unexpected,
            declared_grammar=unexpected,
        )

        errors = self.metadata_errors()

        self.assertEqual(len(errors), 7)
        self.assertTrue(
            all(
                "mismatch: expected" in error and unexpected in error
                for error in errors
            )
        )

    def test_follows_root_input_node_names(self) -> None:
        self.write_metadata(
            maki_node_name="maki-renamed",
            grammar_node_name="tree-sitter-maki-renamed",
        )

        errors = self.metadata_errors()

        self.assertEqual(errors, [])

    def test_reports_flake_only_revision_mismatch(self) -> None:
        unexpected = "3" * 40
        self.write_metadata(flake_maki=unexpected)

        errors = self.metadata_errors()

        self.assertEqual(
            errors,
            [
                "flake.nix Maki declared revision mismatch: "
                f"expected {MAKI_REVISION}, found {unexpected}"
            ],
        )

    def test_rejects_revision_with_a_suffix(self) -> None:
        self.write_metadata(flake_maki=MAKI_REVISION + "BAD")

        errors = self.metadata_errors()

        self.assertEqual(errors, ["flake.nix Maki declared revision is missing"])

    def test_reports_missing_metadata(self) -> None:
        self.write_metadata()
        (self.extension_dir / "extension.toml").write_text(
            "[grammars]\n", encoding="utf-8"
        )

        errors = self.metadata_errors()

        self.assertIn("extension.toml grammar revision is missing", errors)


if __name__ == "__main__":
    unittest.main()
