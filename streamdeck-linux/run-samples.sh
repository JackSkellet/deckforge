#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")"
if [[ ! -x target/debug/deckforge ]]; then
  cargo build --locked
fi
exec ./target/debug/deckforge desktop \
  --plugin ../evidence/official-samples/counter/com.elgato.counter.sdPlugin \
  --plugin ../evidence/plugins/com.elgato.layouts.sdPlugin \
  --state state/live "$@"
