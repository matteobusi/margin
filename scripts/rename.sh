#!/usr/bin/env bash
# Rename the project: replaces the placeholder slug `lzk` everywhere.
#
#   scripts/rename.sh <slug> ["Display Name"]
#
# <slug>: lowercase, e.g. `byzk` or `sorry-free`. Crate names become `<slug>-canon`, ...
# Binary formats do not depend on the name (magic bytes and hash tags are neutral).
set -euo pipefail

slug="${1:-}"
display="${2:-$slug}"
if [[ ! "$slug" =~ ^[a-z][a-z0-9-]*$ ]]; then
  echo "usage: $0 <slug> [\"Display Name\"]   (slug: lowercase letters, digits, dashes)" >&2
  exit 2
fi
cd "$(git rev-parse --show-toplevel)"

ident="${slug//-/_}"                                   # Rust identifiers
upper="$(tr '[:lower:]-' '[:upper:]_' <<<"$slug")"     # constants

files=$(git ls-files -co --exclude-standard \
  | grep -vE '(^|/)Cargo\.lock$|(^|/)\._|\.(png|jpg|ico|wasm|bin|receipt)$' \
  | grep -v '^scripts/rename\.sh$')

for f in $files; do
  [[ -f "$f" ]] || continue
  SLUG="$slug" IDENT="$ident" UPPER="$upper" perl -pi -e '
    s/\blzk_/$ENV{IDENT}_/g;
    s/\bLZK_/$ENV{UPPER}_/g;
    s/\bLZK\b/$ENV{UPPER}/g;
    s/\blzk\b/$ENV{SLUG}/g;
  ' "$f"
done

# Display name in the web app.
DISPLAY="$display" perl -pi -e 's/^(\s*name: ).*$/$1 . "\"$ENV{DISPLAY}\","/e' web/src/brand.ts
perl -pi -e "s|<title>.*</title>|<title>${display//|/}</title>|" web/index.html

echo "Renamed lzk -> $slug (display: $display). Run: cargo test && (cd web && npm run build)"
