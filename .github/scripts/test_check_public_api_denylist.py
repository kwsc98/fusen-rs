#!/usr/bin/env python3

"""Regression tests for the doc-hidden macro ABI source audit."""

import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("check-public-api-denylist.py")
SPEC = importlib.util.spec_from_file_location("check_public_api_denylist", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot import {SCRIPT}")
GATE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GATE)


class MacroV1AllowlistTests(unittest.TestCase):
    def source_with(self, declaration: str = "") -> str:
        exports = ",\n                ".join(sorted(GATE.MACRO_V1_EXPORTS))
        return f"""
#[doc(hidden)]
pub mod __macro {{
    pub mod v1 {{
        pub use crate::{{
            {exports}
        }};
        {declaration}
    }}
}}
"""

    def audit(self, declaration: str = "") -> list[str]:
        with tempfile.TemporaryDirectory() as temporary:
            source_root = Path(temporary)
            source = source_root / "fusen/src/lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text(self.source_with(declaration), encoding="utf-8")
            failures: list[str] = []
            GATE.audit_macro_v1(source_root, failures)
            return failures

    def assert_rejects(self, declaration: str, name: str) -> None:
        failures = self.audit(declaration)
        self.assertTrue(failures, f"expected {declaration!r} to fail the ABI allowlist")
        self.assertIn(name, "\n".join(failures))

    def test_approved_reexports_are_accepted(self) -> None:
        self.assertEqual(self.audit(), [])

    def test_unapproved_public_const_is_rejected(self) -> None:
        self.assert_rejects("pub const UNAPPROVED_CONST: usize = 1;", "UNAPPROVED_CONST")

    def test_other_public_rust_item_forms_are_rejected(self) -> None:
        declarations = {
            "UnexpectedStatic": "pub static mut UnexpectedStatic: usize = 0;",
            "UnexpectedModule": "pub mod UnexpectedModule {}",
            "UnexpectedMacro": "pub macro UnexpectedMacro() {}",
            "UnexpectedExtern": "pub extern crate dependency as UnexpectedExtern;",
            "UnexpectedFunction": 'pub unsafe extern "C" fn UnexpectedFunction() {}',
            "UnexpectedRules": "#[macro_export] macro_rules! UnexpectedRules { () => {} }",
        }
        for name, declaration in declarations.items():
            with self.subTest(name=name):
                self.assert_rejects(declaration, name)


if __name__ == "__main__":
    unittest.main()
