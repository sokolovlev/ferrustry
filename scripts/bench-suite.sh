#!/usr/bin/env bash
# Полный набор бенчмарков Ferrustry для Linux и macOS.
# Использование: scripts/bench-suite.sh [--quick]
# Результат: bench-results/<дата>-<commit>/ (suite.json, machine.txt, perf.txt на Linux)
set -euo pipefail
cd "$(dirname "$0")/.."
commit=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
dir="bench-results/$(date +%Y-%m-%d_%H%M)-$commit"
mkdir -p "$dir"
cargo build --release -p ferrustry-headless
bin=target/release/ferrustry-headless
{
  echo "commit: $commit"
  echo "rustc: $(rustc --version)"
  echo "os: $(uname -srm)"
  if command -v lscpu >/dev/null; then lscpu | grep -E 'Model name|^CPU\(s\)|MHz|L1d|L2|L3'; fi
  if [ "$(uname)" = Darwin ]; then sysctl -n machdep.cpu.brand_string; sysctl hw.ncpu hw.memsize; fi
  if [ -r /proc/meminfo ]; then grep MemTotal /proc/meminfo; fi
} > "$dir/machine.txt"
FERRUSTRY_COMMIT=$commit "$bin" suite --runs 3 --out "$dir/suite.json" "$@"
if command -v perf >/dev/null && [ "$(uname)" = Linux ]; then
  for s in b2-comb b3; do
    echo "== $s 100k" >> "$dir/perf.txt"
    perf stat -e cycles,instructions,cache-references,cache-misses,branch-misses,L1-dcache-load-misses \
      "$bin" bench "$s" --conveyors 100000 --ticks 3000 --warmup 6000 >/dev/null 2>> "$dir/perf.txt" || \
      echo "perf stat failed (try: sudo sysctl kernel.perf_event_paranoid=1)" >> "$dir/perf.txt"
  done
fi
echo "Готово: $dir"
