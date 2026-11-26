#!/usr/bin/env bash
# Benchmark Margin proving on an NVIDIA GPU (for example a rented RTX 4090).
#
# On a fresh Ubuntu 22.04/24.04 machine with the NVIDIA driver and the CUDA toolkit
# (nvcc, 12.x) installed, as most GPU rental images provide, plus build tools:
#
#   apt-get install -y build-essential git curl pkg-config libssl-dev protobuf-compiler
#
#   curl -fsSL https://raw.githubusercontent.com/matteobusi/margin/main/scripts/gpu-bench.sh | bash
#
# It builds `margin` with CUDA, embedding the released checker (same image ID as v0.0.1,
# no Docker needed), proves the two benchmark inputs in bench/, verifies the receipts and
# prints cycles per second. Building the CUDA kernels takes 10-20 minutes the first time.
set -euo pipefail

VERSION="v0.0.1"
REPO="matteobusi/margin"
WORK="${WORK:-$HOME/margin-bench}"

say() { printf '\n==> %s\n' "$*"; }
command -v nvidia-smi >/dev/null || { echo "nvidia-smi not found: no NVIDIA driver?"; exit 1; }
command -v nvcc >/dev/null || { echo "nvcc not found: install the CUDA toolkit (12.x)"; exit 1; }
for t in cc git pkg-config protoc; do
  command -v "$t" >/dev/null || { echo "$t not found: apt-get install -y build-essential git pkg-config libssl-dev protobuf-compiler"; exit 1; }
done
# Debug info roughly doubles the build output; not needed here.
export CARGO_PROFILE_RELEASE_DEBUG="${CARGO_PROFILE_RELEASE_DEBUG:-false}"
nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader
nvcc --version | tail -1
nproc; free -g | sed -n 2p

say "Rust and the RISC Zero toolchain"
if ! command -v cargo >/dev/null; then
  curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
. "$HOME/.cargo/env"
if [ ! -x "$HOME/.risc0/bin/rzup" ]; then
  curl -fsSL https://risczero.com/install | bash
fi
export PATH="$HOME/.risc0/bin:$PATH"
rzup install >/dev/null

say "Source and the released checker"
mkdir -p "$WORK"; cd "$WORK"
[ -d margin ] || git clone -q "https://github.com/$REPO" margin
git -C margin pull -q
curl -fsSL -o margin-guest.bin "https://github.com/$REPO/releases/download/$VERSION/margin-guest.bin"
(cd margin/zkvm && MARGIN_PREBUILT_GUEST="$WORK/margin-guest.bin" \
  cargo build --release -p margin-host --features kernel-nanoda,cuda)
M="$WORK/margin/zkvm/target/release/margin"
echo "image id: $("$M" image-id)"

say "Proving"
for b in double_comm primes; do
  in="$WORK/margin/bench/$b.input"
  start=$(date +%s.%N)
  "$M" prove "$in" "$WORK/$b.receipt" 2>&1 | grep -E "cycles|time"
  end=$(date +%s.%N)
  "$M" verify "$WORK/$b.receipt" | head -1
  cycles=$("$M" execute "$in" 2>&1 >/dev/null | sed -n 's/.*~\([0-9]*\) cycles.*/\1/p')
  awk -v c="$cycles" -v s="$start" -v e="$end" -v b="$b" \
    'BEGIN { printf "RESULT %s: %d cycles (padded) in %.1f s = %.0f cycles/s\n", b, c, e - s, c / (e - s) }'
done
say "Composition: module receipts combined into one receipt (bench/compose)"
C="$WORK/margin/bench/compose"
(cd margin/zkvm && MARGIN_PREBUILT_GUEST="$C/guest.elf" \
  cargo build --release -p margin-host --features kernel-nanoda,cuda)
id="$("$M" image-id)"
[ "$id" = "4022792efeedfabee594d66a70893b5972199e53c48802d3ad556f28b4debf85" ] || { echo "unexpected image id $id"; exit 1; }
"$M" prove "$C/A.input" "$WORK/A.receipt" 2>&1 | grep -E "proved|rror"
"$M" prove --assume "$WORK/A.receipt" "$C/B.input" "$WORK/B.receipt" 2>&1 | grep -E "proved|rror"
"$M" prove --assume "$WORK/A.receipt,$WORK/B.receipt" "$C/final.input" "$WORK/final.receipt" 2>&1 | grep -E "proved|rror"
if "$M" verify --checkers "$C/checkers.json" --envs "$C/envs.json" "$WORK/final.receipt"; then
  echo "RESULT composition: VALID ($(stat -c %s "$WORK/final.receipt") bytes)"
else
  echo "RESULT composition: FAILED"
fi
# Negative check: the final run cannot be proved without its module receipts.
if "$M" prove --assume "$WORK/A.receipt" "$C/final.input" "$WORK/bad.receipt" >/dev/null 2>&1; then
  echo "RESULT composition-negative: UNEXPECTEDLY PROVED"
else
  echo "RESULT composition-negative: rejected as expected (missing module receipt)"
fi

say "Done. Please send back the RESULT lines and the GPU name above."
