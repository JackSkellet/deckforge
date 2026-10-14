#!/usr/bin/env python3
"""A terminal input sink. Received text is data and is never executed as a shell command."""
import json,sys
from pathlib import Path
root=Path(sys.argv[1]);assert str(root).startswith('/tmp/deckforge-focused-')
assert sys.stdin.isatty()
(root/'terminal-ready.json').write_text(json.dumps({'tty':True}))
print('Deckforge isolated paste receiver',flush=True)
lines=[sys.stdin.readline(),sys.stdin.readline()]
temporary=root/'terminal-result.tmp';temporary.write_text(json.dumps({'tty':True,'lines':lines}));temporary.replace(root/'terminal-result.json')
