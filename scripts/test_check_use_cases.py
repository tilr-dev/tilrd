#!/usr/bin/env python3
"""Exercise the checker CLI with real source and Markdown fixtures."""

from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


CHECKER = Path(__file__).with_name('check_use_cases.py')


class References(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.write('crates/demo/tests/reconnect.rs', '''
#[cfg(feature = "failpoints")]
#[tokio::test]
async fn reconnects() {}
mod nested { #[test] fn gated() {} }
// fn only_in_a_comment() {}
/* outer
   /* nested comment */
   fn only_in_a_nested_comment() {}
*/
const EXAMPLE: &str = r#"fn only_in_a_string() {}"#;
''')
        self.write('crates/demo/src/title.rs', '''
#[cfg(test)]
mod tests { #[test] fn title_follows_pane() {} }
''')
        self.write('crates/demo/src/conn/mod.rs', '''
mod bundled_tls_tests { #[test] fn trusts_roots() {} }
''')
        self.write('itests/scenarios/uc_123.py', 'async def reconnects():\n    pass\n')

    def write(self, name, contents):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents)

    def run_checker(self, markdown):
        self.write('USE_CASES.md', markdown)
        return subprocess.run(
            [sys.executable, str(CHECKER), str(self.root)],
            text=True, capture_output=True, check=False,
        )

    def test_existing_references_and_annotations(self):
        result = self.run_checker('''# Coverage
| Use case | Test |
| --- | --- |
| A `pipe | in code` | `reconnects`, `reconnect::reconnects` (`failpoints`) |
| Gated | `reconnect::nested::gated` (Linux) |
| Unit | `title::tests::title_follows_pane`, `conn::bundled_tls_tests::trusts_roots` |
| Python | `uc_123.py::reconnects` |

| Case | Check |
| --- | --- |
| This column is not named Test | `not_a_test_reference` |
''')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_missing_references_fail_with_their_line(self):
        for name in ['misspelled', 'reconnect::missing',
                     'wrong::tests::title_follows_pane', 'title::tests::missing',
                     'uc_123.py::missing', 'uc_999.py::reconnects',
                     'only_in_a_comment', 'only_in_a_nested_comment',
                     'only_in_a_string']:
            with self.subTest(name=name):
                result = self.run_checker(
                    f'| Use case | Test |\n| --- | --- |\n| Missing | `{name}` |\n'
                )
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn(f'USE_CASES.md:3: unknown test `{name}`', result.stderr)

    def test_test_column_can_move_and_tables_do_not_leak(self):
        result = self.run_checker('''| Test | Use case |
| --- | --- |
| `reconnects` | `not_a_reference` |

| Other | Description |
| --- | --- |
| `also_not_a_reference` | text |
''')
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == '__main__':
    unittest.main()
