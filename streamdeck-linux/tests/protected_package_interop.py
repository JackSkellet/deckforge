#!/usr/bin/env python3
"""Check protected import gates using explicitly supplied local vendor artifacts.

Manifest authentication is implemented. Package file verification is unfinished;
this test must not mistake successful manifest decryption for a valid installation.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

PACKAGE_HASH = '9d063e9b3616693093e565f762cdc65bd5638bdbb9f42e535e1c738b429302cd'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--package', type=Path, required=True)
    parser.add_argument('--reference-app', type=Path, required=True)
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    binary, package, reference = (path.resolve(strict=True) for path in (args.binary, args.package, args.reference_app))
    assert hashlib.sha256(package.read_bytes()).hexdigest() == PACKAGE_HASH
    report = {'package_sha256': PACKAGE_HASH, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'reference_sha256': hashlib.sha256(reference.read_bytes()).hexdigest(), 'installation_completed': False}
    with tempfile.TemporaryDirectory(prefix='deckforge-protected-') as directory:
        root = Path(directory)
        wrong = root/'wrong.exe'; wrong.write_bytes(b'not the original app')
        for label, ref, expected in [('absent', None, 'Elgato-protected'),
                                     ('wrong', wrong, 'Unsupported original'),
                                     ('original', reference, 'file-integrity verification')]:
            target = root/label
            command = [str(binary), 'install', str(package), '--destination', str(target)]
            if ref:
                command += ['--reference-app', str(ref)]
            result = subprocess.run(command, capture_output=True, text=True, timeout=30)
            assert result.returncode != 0 and expected in result.stderr, result.stderr
            assert not target.exists(), 'Rejected package left partial installation'
            report[label] = result.stderr.strip()
    assert hashlib.sha256(package.read_bytes()).hexdigest() == PACKAGE_HASH
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2)+'\n')
    print('PASS: absent/wrong reference rejected; authenticated manifest stops at unresolved package-integrity gate; no installation or plugin execution.')


if __name__ == '__main__':
    main()
