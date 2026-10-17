#!/usr/bin/env python3
"""Copy a dependency-free connector starter; never overwrite a destination."""
import argparse
from pathlib import Path
import re
import shutil
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--language', choices=('python', 'typescript'), required=True)
    parser.add_argument('--name', required=True, help='source identifier (1–128 ASCII token characters)')
    parser.add_argument('--output', type=Path, required=True, help='new directory; must not exist')
    args = parser.parse_args()
    if not re.fullmatch(r'[A-Za-z0-9_.:/-]{1,128}', args.name):
        parser.error('name must contain 1–128 ASCII letters, digits, or -_.:/')
    templates = Path(__file__).resolve().parents[1] / 'connectors' / 'templates'
    extension = 'py' if args.language == 'python' else 'ts'
    files = {
        f'connector.{extension}': (templates / args.language / f'connector.{extension}').read_text(),
        'fixture.ndjson': (templates / 'fixture.ndjson').read_text(),
        'README.md': (templates / 'README.md').read_text(),
    }
    # mkdir is the overwrite guard, including existing empty directories and symlinks.
    try:
        args.output.mkdir(parents=True, exist_ok=False)
    except OSError as error:
        parser.exit(1, f'cannot create destination: {error}\n')
    try:
        for name, content in files.items():
            (args.output / name).write_text(content.replace('__SOURCE__', args.name))
    except OSError:
        shutil.rmtree(args.output)
        raise
    print(f'Created {args.language} connector in {args.output}', file=sys.stderr)


if __name__ == '__main__':
    main()
