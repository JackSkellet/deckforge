#!/usr/bin/env python3
"""Build the pinned GPL OBS bridge against local OBS/Qt6; never starts OBS.

Requires git, cmake, ninja, a C++ compiler, OBS development files and Qt6 Widgets.
Output includes the complete corresponding source and build wrapper.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

PROJECT = Path(__file__).resolve().parents[1]
REVISION = 'fcddcde567bdeafd235829d4fe8519894b158e29'
REPOSITORY = 'https://github.com/elgatosf/streamdeck-obs-plugin2.git'

def run(*args, cwd=None):
    subprocess.run(list(args), cwd=cwd, check=True)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=PROJECT / 'target/release/obs-bridge')
    args = parser.parse_args()
    source = PROJECT / 'target/obs-bridge-source'
    build = PROJECT / 'target/obs-bridge-linux'
    if not source.exists():
        run('git', 'clone', '--no-checkout', REPOSITORY, str(source))
        run('git', 'checkout', '--detach', REVISION, cwd=source)
    head = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
    if head != REVISION:
        raise SystemExit(f'Expected pinned OBS bridge {REVISION}; checkout has {head}. Use a fresh target/obs-bridge-source checkout.')
    if subprocess.check_output(['git', '-C', str(source), 'status', '--porcelain', '--untracked-files=no'], text=True).strip():
        raise SystemExit('OBS bridge checkout must be clean.')
    run('git', 'checkout', '--detach', REVISION, cwd=source)
    run('git', 'submodule', 'update', '--init', '--recursive', cwd=source)
    run('cmake', '-S', str(PROJECT/'scripts/obs-bridge'), '-B', str(build), '-G', 'Ninja',
        f'-DUPSTREAM={source}', '-DCMAKE_BUILD_TYPE=Release')
    run('cmake', '--build', str(build), '--parallel', str(min(os.cpu_count() or 2, 4)))
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    shutil.copy2(build/'StreamDeckPlugin.so', output/'StreamDeckPlugin.so')
    shutil.copytree(source/'data', output/'data', dirs_exist_ok=True)
    for name in ('LICENSE.md', 'LICENSES.md'):
        shutil.copy2(source/name, output/name)
    # Include submodule contents; exclude only git internals and caches.
    with tarfile.open(output/'source.tar.gz', 'w:gz') as archive:
        def source_only(entry):
            return None if '.git' in Path(entry.name).parts else entry
        archive.add(source, arcname='upstream', filter=source_only)
        archive.add(PROJECT/'scripts/obs-bridge', arcname='linux-build')
    (output/'build.json').write_text(json.dumps({
        'repository': REPOSITORY, 'revision': REVISION, 'version': '5.5.4.0-deckforge-linux',
        'sha256': hashlib.sha256((output/'StreamDeckPlugin.so').read_bytes()).hexdigest(),
        'build': 'cmake -S linux-build -B build -DUPSTREAM="$PWD/upstream" -DCMAKE_BUILD_TYPE=Release && cmake --build build',
    }, indent=2)+'\n')
    print(output)

if __name__ == '__main__':
    main()
