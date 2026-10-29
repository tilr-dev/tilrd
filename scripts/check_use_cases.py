#!/usr/bin/env python3
"""Validate backticked test names in USE_CASES.md's Test columns.

Run directly, or through the Cargo use_cases integration suite (and bin/check).
Rust references resolve to function declarations in crates/*/{tests,src}; Python
references use uc_NNN.py::function in itests/scenarios. Feature/platform gates do
not remove declarations from this source check. `failpoints` is an annotation,
not a reference. Other coverage tools are documented outside the Test column.
"""

import ast
from pathlib import Path
import re
import sys


# Skip comments and literals before counting module braces or finding functions.
RUST_TOKEN = re.compile(
    r'//[^\n]*|(?P<block>/\*)|(?:br|r)(?P<hashes>\#*)".*?"(?P=hashes)'
    r'|b?"(?:\\.|[^"\\])*"|b?\'(?:\\.|[^\'\\])\''
    r'|\bmod\s+(?P<module>\w+)\s*\{'
    r'|\bfn\s+(?P<function>\w+)\b|(?P<brace>[{}])',
    re.DOTALL,
)


def rust_functions(path, prefix):
    scopes = []
    source = path.read_text()
    position = 0
    while token := RUST_TOKEN.search(source, position):
        position = token.end()
        if token['block']:
            # Rust block comments nest, including documentation comments.
            depth = 1
            for delimiter in re.finditer(r'/\*|\*/', source[position:]):
                depth += 1 if delimiter[0] == '/*' else -1
                if depth == 0:
                    position += delimiter.end()
                    break
            else:
                position = len(source)
        elif token['module']:
            scopes.append(token['module'])
        elif token['brace'] == '{':
            scopes.append(None)
        elif token['brace'] == '}':
            if scopes:
                scopes.pop()
        elif token['function']:
            parts = prefix + [name for name in scopes if name] + [token['function']]
            yield '::'.join(parts)


def known_names(root):
    names = set()
    crates = {path.name for path in (root / 'crates').iterdir() if path.is_dir()}
    for crate in sorted(crates):
        base = root / 'crates' / crate
        for path in sorted((base / 'tests').glob('*.rs')):
            for name in rust_functions(path, [path.stem]):
                names.add(name)
                names.add(name.split('::')[-1])
        for path in sorted((base / 'src').rglob('*.rs')):
            parts = list(path.relative_to(base / 'src').with_suffix('').parts)
            if parts[-1] in {'lib', 'main', 'mod'}:
                parts.pop()
            for name in rust_functions(path, parts):
                names.add(name)
                # Some existing tables introduce the source file in prose and
                # use only the function name, especially the shared TLS crate.
                names.add(name.split('::')[-1])
    for path in sorted((root / 'itests' / 'scenarios').glob('uc_*.py')):
        tree = ast.parse(path.read_text(), filename=str(path))
        for node in tree.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                names.add(f'{path.name}::{node.name}')
    return names


def check(root):
    names = known_names(root)
    errors = []
    test_column = None
    for number, line in enumerate((root / 'USE_CASES.md').read_text().splitlines(), 1):
        line = line.strip()
        if not line.startswith('|'):
            test_column = None
            continue
        # A pipe in an inline code span belongs to that cell, not the table.
        cells = [cell.strip() for cell in re.split(
            r'(?<!\\)\|(?=(?:[^`]*`[^`]*`)*[^`]*$)', line.strip('|')
        )]
        if 'Test' in cells:
            test_column = cells.index('Test')
            continue
        if test_column is None or test_column >= len(cells):
            continue
        for name in re.findall(r'`([^`]+)`', cells[test_column]):
            if name == 'failpoints':
                continue
            if name not in names:
                errors.append(f'USE_CASES.md:{number}: unknown test `{name}`')
    return errors


def main():
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[1]
    errors = check(root)
    if errors:
        print('\n'.join(errors), file=sys.stderr)
        return 1
    print('USE_CASES.md test references: ok')
    return 0


if __name__ == '__main__':
    sys.exit(main())
