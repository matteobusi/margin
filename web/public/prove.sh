#!/usr/bin/env bash
# margin prove.sh: prove Lean 4 theorems in zero knowledge, on your own machine.
#
# Run it in the root of your Lake project:
#
#   bash prove.sh MyProject.Basic my_theorem [more_theorems...]
#
# What it does (nothing leaves your machine; downloads go to ~/.margin):
#   1. checks that your project uses a supported Lean / Mathlib version;
#   2. downloads the margin CLI for your platform and checks its SHA-256;
#   3. builds lean4export for your Lean version (once);
#   4. gets the environment cache for your Lean / Mathlib version (once);
#   5. builds your project, exports the theorems, pre-checks them natively and
#      estimates the proving time;
#   6. proves them in the zkVM and writes a receipt to ./margin-out/.
#
# Options:
#   --check-only     stop after the pre-check and the cycle estimate (no proof)
#   --anonymous      no owner key: the receipt is anonymous and nobody (not even
#                    you) can ever claim authorship
#   --key FILE       owner key (default ~/.margin/owner.key, created on first use)
#   --out DIR        output directory (default ./margin-out)
#   --open-theorems  open library theorems in full (only if the pre-check asks)
#   -y, --yes        do not ask before starting a long proof
#
# Environment overrides: MARGIN_HOME (default ~/.margin), MARGIN_BIN (use this margin binary),
# MARGIN_LEAN4EXPORT (use this lean4export binary).
set -euo pipefail

# ---- pinned versions ---------------------------------------------------------------
MARGIN_VERSION="v0.0.1"
MARGIN_REPO="matteobusi/margin"
# SHA-256 of the release binaries. Filled in when a release is published; while empty,
# the script checks against the release's SHA256SUMS file instead (and says so).
SHA_LINUX_X86_64="d506c09d0b188ad2fcee5db7f7f780ac0f9babfaaa41a5ccd142170dad2da02f"
SHA_MACOS_ARM64="ec7da80853c7a4a67c2bd56a2629fd43ecd34b44ac6e90f78ca384541119c971"
LEAN4EXPORT_URL="https://github.com/leanprover/lean4export"
LEAN4EXPORT_REV="df81913eee5519d5a88ae5b0fd701b5298cc29ab"
# Supported environments, one per line:
#   id | Lean toolchain | Mathlib commit ("-" = core Lean only) | env_root | export root module
# Must agree with envs.json (checked by web/scripts/check-prove-sh.mjs at build time).
ENVS="
mathlib-5d3d31c|leanprover/lean4:v4.37.0-rc1|5d3d31c50c7b58ed18150f0b58034c12cf3c6afa|fb4484bb90919eaa79dd32ec980663284e6c09ca2bbec02bf00723ad8f727a67|Mathlib
lean-v4.36.0-init|leanprover/lean4:v4.36.0|-|275127d992d75535efe7076f5b501dfed0faf59fca4d233c90c73220492bbb19|Init
"
# Rough CPU proving speed, for the estimate only (cycles per second).
CYCLES_PER_SEC=8000

# ---- helpers -----------------------------------------------------------------------
if [ -t 2 ]; then B=$'\033[1m'; D=$'\033[2m'; R=$'\033[31m'; G=$'\033[32m'; Y=$'\033[33m'; N=$'\033[0m'
else B=""; D=""; R=""; G=""; Y=""; N=""; fi
step() { printf '\n%s==> %s%s\n' "$B" "$*" "$N" >&2; }
info() { printf '    %s\n' "$*" >&2; }
warn() { printf '%s    warning: %s%s\n' "$Y" "$*" "$N" >&2; }
die()  { printf '%serror: %s%s\n' "$R" "$*" "$N" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed. $2"; }
sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
human_secs() { local s=$1; if [ "$s" -lt 120 ]; then echo "${s} s"; elif [ "$s" -lt 7200 ]; then echo "$((s / 60)) min"; else echo "$((s / 3600)) h $(((s % 3600) / 60)) min"; fi; }
usage() { sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

# ---- arguments ---------------------------------------------------------------------
MARGIN_HOME="${MARGIN_HOME:-$HOME/.margin}"
CHECK_ONLY=0; ANON=0; YES=0; OPEN=""; KEY="$MARGIN_HOME/owner.key"; OUT="margin-out"; POS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --check-only) CHECK_ONLY=1 ;;
    --anonymous) ANON=1 ;;
    --open-theorems) OPEN="--open-theorems" ;;
    -y|--yes) YES=1 ;;
    --key) [ $# -ge 2 ] || die "--key needs a file"; KEY="$2"; shift ;;
    --out) [ $# -ge 2 ] || die "--out needs a directory"; OUT="$2"; shift ;;
    -h|--help) usage 0 ;;
    -*) die "unknown option $1 (see --help)" ;;
    *) POS+=("$1") ;;
  esac
  shift
done
[ ${#POS[@]} -ge 2 ] || { printf 'usage: bash prove.sh [options] <Module> <theorem>...\n\n' >&2; usage 2; }
MODULE="${POS[0]}"; THEOREMS=("${POS[@]:1}")

# ---- 1. project --------------------------------------------------------------------
step "Checking your project"
[ -f lean-toolchain ] || die "no lean-toolchain here. Run this script in the root of your Lake project."
[ -f lakefile.lean ] || [ -f lakefile.toml ] || die "no lakefile.lean or lakefile.toml here. Run this script in the root of your Lake project."
need git ""; need curl ""; need lake "Install Lean with elan: https://lean-lang.org/install/"
TOOLCHAIN="$(tr -d '[:space:]' < lean-toolchain)"
info "Lean toolchain: $TOOLCHAIN"

MATHLIB_REV="-"
if [ -f lake-manifest.json ]; then
  obj="$(awk 'p { obj = obj $0 "\n"; if ($0 ~ /\}/) { if (obj ~ /"name": *"mathlib"/) printf "%s", obj; obj = "" } } /"packages"/ { p = 1 }' lake-manifest.json)"
  if [ -n "$obj" ]; then
    rev="$(printf '%s' "$obj" | sed -n 's/.*"rev": *"\([0-9a-f]*\)".*/\1/p')"
    dir="$(printf '%s' "$obj" | sed -n 's/.*"dir": *"\([^"]*\)".*/\1/p')"
    if [ -n "$rev" ]; then MATHLIB_REV="$rev"
    elif [ -n "$dir" ]; then MATHLIB_REV="$(git -C "$dir" rev-parse HEAD 2>/dev/null)" || die "cannot read the Mathlib commit in $dir"
    else die "cannot find the Mathlib commit in lake-manifest.json"; fi
  fi
fi
[ "$MATHLIB_REV" = "-" ] && info "Mathlib: not used" || info "Mathlib: $MATHLIB_REV"

ENV_LINE=""
while IFS='|' read -r id tc ml root mod; do
  [ -n "$id" ] || continue
  if [ "$tc" = "$TOOLCHAIN" ] && [ "$ml" = "$MATHLIB_REV" ]; then ENV_LINE="$id|$root|$mod"; fi
done <<< "$ENVS"
if [ -z "$ENV_LINE" ]; then
  printf '%serror: this Lean / Mathlib version is not supported (yet).%s\n\nSupported versions:\n' "$R" "$N" >&2
  while IFS='|' read -r id tc ml root mod; do
    [ -n "$id" ] || continue
    if [ "$ml" = "-" ]; then printf '  %-28s core Lean only\n' "$tc" >&2
    else printf '  %-28s Mathlib %s\n' "$tc" "$ml" >&2; fi
  done <<< "$ENVS"
  printf '\nSet lean-toolchain (and the Mathlib rev in your lakefile) to one of these, then run\n`lake update` and try again.\n' >&2
  exit 1
fi
IFS='|' read -r ENV_ID ENV_ROOT ENV_MODULE <<< "$ENV_LINE"
info "environment: $ENV_ID (root ${ENV_ROOT:0:16}…)"
mkdir -p "$MARGIN_HOME/bin" "$MARGIN_HOME/envs" "$MARGIN_HOME/lean4export"

# ---- 2. margin ------------------------------------------------------------------------
step "Getting margin $MARGIN_VERSION"
if [ -n "${MARGIN_BIN:-}" ]; then
  MARGIN="$MARGIN_BIN"; info "using MARGIN_BIN=$MARGIN"
else
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) ASSET="margin-linux-x86_64"; SUMS="SHA256SUMS-linux"; PIN="$SHA_LINUX_X86_64" ;;
    Darwin-arm64) ASSET="margin-macos-arm64"; SUMS="SHA256SUMS-macos"; PIN="$SHA_MACOS_ARM64" ;;
    *) die "no prebuilt margin for $(uname -s) $(uname -m). Build it from source (see the README), then set MARGIN_BIN." ;;
  esac
  MARGIN="$MARGIN_HOME/bin/margin-$MARGIN_VERSION"
  if [ ! -x "$MARGIN" ]; then
    base="https://github.com/$MARGIN_REPO/releases/download/$MARGIN_VERSION"
    info "downloading $base/$ASSET"
    curl -fL --progress-bar -o "$MARGIN.part" "$base/$ASSET" || die "download failed"
    if [ -z "$PIN" ]; then
      warn "no pinned checksum in this script; using the release's $SUMS"
      PIN="$(curl -fsSL "$base/$SUMS" | awk -v a="$ASSET" '$2 == a { print $1 }')"
      [ -n "$PIN" ] || die "cannot read the checksum of $ASSET from $SUMS"
    fi
    [ "$(sha256 "$MARGIN.part")" = "$PIN" ] || { rm -f "$MARGIN.part"; die "checksum mismatch for $ASSET"; }
    chmod +x "$MARGIN.part"; mv "$MARGIN.part" "$MARGIN"
  fi
  ln -sf "margin-$MARGIN_VERSION" "$MARGIN_HOME/bin/margin"
fi
info "checker image ID: $("$MARGIN" image-id)"

# ---- 3. lean4export ----------------------------------------------------------------
step "Getting lean4export for $TOOLCHAIN"
if [ -n "${MARGIN_LEAN4EXPORT:-}" ]; then
  L4E="$MARGIN_LEAN4EXPORT"; info "using MARGIN_LEAN4EXPORT=$L4E"
else
  L4E_DIR="$MARGIN_HOME/lean4export/$(printf '%s' "$TOOLCHAIN" | tr '/:' '__')"
  L4E="$L4E_DIR/.lake/build/bin/lean4export"
  if [ ! -x "$L4E" ]; then
    info "building lean4export ${LEAN4EXPORT_REV:0:7} (a few minutes, once)"
    rm -rf "$L4E_DIR"
    git clone -q "$LEAN4EXPORT_URL" "$L4E_DIR"
    git -C "$L4E_DIR" -c advice.detachedHead=false checkout -q "$LEAN4EXPORT_REV"
    printf '%s\n' "$TOOLCHAIN" > "$L4E_DIR/lean-toolchain"
    (cd "$L4E_DIR" && lake build lean4export >&2) || die "building lean4export failed"
  fi
  info "$L4E"
fi
export MARGIN_LEAN4EXPORT="$L4E"

# ---- 4. environment cache ----------------------------------------------------------
step "Getting the environment cache ($ENV_ID)"
CACHE="$MARGIN_HOME/envs/$ENV_ID.cache"
env_root_of() { "$MARGIN" env root "$1" 2>/dev/null | awk '/env_root:/ { print $2 }'; }
if [ ! -f "$CACHE" ]; then
  url="https://github.com/$MARGIN_REPO/releases/download/$MARGIN_VERSION/$ENV_ID.cache"
  info "trying $url"
  if curl -fL --progress-bar -o "$CACHE.part" "$url"; then
    info "checking the downloaded cache against the published root (1-2 min)"
    got="$(env_root_of "$CACHE.part")"
    [ "$got" = "$ENV_ROOT" ] || { rm -f "$CACHE.part"; die "downloaded cache has root $got, expected $ENV_ROOT"; }
  else
    rm -f "$CACHE.part"
    info "not available for download; building it from your installed $ENV_MODULE (once)."
    [ "$ENV_MODULE" = "Mathlib" ] && info "Mathlib: about 30 min, 13 GB RAM and 8 GB of temporary disk space."
    [ "$ENV_MODULE" = "Mathlib" ] && { lake exe cache get >&2 || warn "lake exe cache get failed; lake build may take long"; }
    [ "$ENV_MODULE" = "Mathlib" ] && { lake build Mathlib >&2 || die "lake build Mathlib failed"; }
    tmp="$MARGIN_HOME/envs/$ENV_ID.ndjson.part"
    lake env "$L4E" "$ENV_MODULE" > "$tmp" || { rm -f "$tmp"; die "lean4export $ENV_MODULE failed"; }
    got="$("$MARGIN" env cache "$tmp" "$CACHE.part" | awk '/env_root:/ { print $2 }')"
    rm -f "$tmp"
    [ "$got" = "$ENV_ROOT" ] || { rm -f "$CACHE.part"; die "your $ENV_MODULE gives root $got, expected $ENV_ROOT. Is your checkout modified?"; }
  fi
  mv "$CACHE.part" "$CACHE"
fi
info "$CACHE"

# ---- 5. owner key ------------------------------------------------------------------
KEY_ARGS=()
if [ "$ANON" = 1 ]; then
  step "Owner key: none (anonymous receipt)"
else
  step "Owner key"
  if [ ! -f "$KEY" ]; then
    mkdir -p "$(dirname "$KEY")"
    "$MARGIN" keygen "$KEY" > "$KEY.pub.txt"
    info "created $KEY (keep it secret, back it up: it is how you claim authorship later)"
  fi
  [ -f "$KEY.pub.txt" ] && info "$(grep 'public key' "$KEY.pub.txt")"
  KEY_ARGS=(--key "$KEY")
fi

# ---- 6. export, pre-check, estimate ------------------------------------------------
step "Building your project"
lake build >&2 || die "lake build failed. Fix the errors above first."

NAME="${THEOREMS[0]}"; WORK="$OUT/$NAME"
mkdir -p "$OUT"
step "Exporting ${THEOREMS[*]} from $MODULE"
"$MARGIN" export "$MODULE" "$WORK.export.ndjson" "${THEOREMS[@]}"

step "Pre-checking (native, no proof)"
"$MARGIN" prepare ${KEY_ARGS[@]+"${KEY_ARGS[@]}"} $OPEN "$CACHE" "$WORK.export.ndjson" "$WORK.input.bin" "${THEOREMS[@]}"
"$MARGIN" check "$WORK.input.bin" | tee "$WORK.statement.txt" >&2

step "Estimating the proof size"
cycles="$("$MARGIN" execute "$WORK.input.bin" 2>&1 >/dev/null | sed -n 's/.*~\([0-9]*\) cycles.*/\1/p')"
[ -n "$cycles" ] || die "cycle estimate failed"
secs=$((cycles / CYCLES_PER_SEC))
info "$cycles cycles: roughly $(human_secs "$secs") on a 16-core CPU, about 10 GB of RAM"
mem_gb=""
if [ -r /proc/meminfo ]; then mem_gb=$(( $(awk '/MemTotal/ { print $2 }' /proc/meminfo) / 1048576 ))
elif command -v sysctl >/dev/null 2>&1; then mem_gb=$(( $(sysctl -n hw.memsize) / 1073741824 )); fi
[ -n "$mem_gb" ] && [ "$mem_gb" -lt 12 ] && warn "this machine has ${mem_gb} GB of RAM; proving needs about 10 GB"

if [ "$CHECK_ONLY" = 1 ]; then
  printf '\n%sPre-check passed.%s Run again without --check-only to prove.\n' "$G" "$N" >&2
  exit 0
fi

# ---- 7. prove ----------------------------------------------------------------------
if [ "$YES" = 0 ] && [ -t 0 ]; then
  printf '\nStart proving now? [Y/n] ' >&2; read -r ans
  case "$ans" in n*|N*) info "stopped. Your input is in $WORK.input.bin; prove later with: $MARGIN prove $WORK.input.bin $WORK.receipt"; exit 0 ;; esac
fi
step "Proving (keep this terminal open)"
"$MARGIN" prove "$WORK.input.bin" "$WORK.receipt"
step "Verifying the receipt"
"$MARGIN" verify "$WORK.receipt" >&2
rm -f "$WORK.input.bin"

printf '\n%sDone.%s Your receipt: %s\n\n' "$G" "$N" "$WORK.receipt" >&2
cat >&2 <<EOF
Next:
  - Publish $WORK.receipt. It shows the statements above and hides your proofs.
  - Anyone can verify it on the website (Verify page) or with: margin verify $NAME.receipt
  - Also publish a statement file: the theorems with \`:= by sorry\`, so others can run
    \`margin match\` against their own copy.
EOF
[ "$ANON" = 0 ] && printf '  - To claim authorship later: %s claim --note "your name" %s %s\n' "$MARGIN" "$WORK.receipt" "$KEY" >&2
[ "$ANON" = 0 ] && printf '    Keep %s safe: without it you cannot claim this receipt.\n' "$KEY" >&2
true
