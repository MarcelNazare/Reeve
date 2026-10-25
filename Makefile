# Reeve — build helpers and benchmark harness
#
# Windows: run from Git Bash, MSYS2, or any POSIX shell that has `make`.
# From PowerShell, edit the `run` target as noted below.

CARGO ?= cargo

.PHONY: all build run check test fmt clippy release clean distclean \
        bench bench-scenario bench-reeve bench-electron bench-chrome \
        bench-quick bench-long

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

all: build

build:
	$(CARGO) build

release:
	$(CARGO) build --release

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------
# NOTE: `RUST_LOG=... $(CARGO) ...` is POSIX shell syntax. From PowerShell
# instead, replace with:  $$env:RUST_LOG="info,reeve=debug"; $(CARGO) run

run:
	RUST_LOG=info,reeve=debug $(CARGO) run

# ---------------------------------------------------------------------------
# Checks
# ---------------------------------------------------------------------------

check:
	$(CARGO) check

test:
	$(CARGO) test

fmt:
	$(CARGO) fmt

clippy:
	$(CARGO) clippy -- -D warnings

# ---------------------------------------------------------------------------
# Benchmark
# ---------------------------------------------------------------------------
# All targets depend on a release build so the measured binary is optimized.
# Override defaults per-invocation:
#   make bench BENCH_DURATION=300
#   make bench BENCH_OUT=benchmark-results/2025-10

BENCH           := $(CARGO) run --release --bin benchmark --
BENCH_DURATION  ?= 120
BENCH_OUT       ?= benchmark-results

# Full suite: Electron + Reeve + Chrome tabs, ~(duration * 3) total
bench: release
	$(BENCH) --scenario all --duration $(BENCH_DURATION) --out $(BENCH_OUT)

# Single scenario: make bench-scenario SCENARIO=reeve
bench-scenario: release
	@test -n "$(SCENARIO)" || (echo "Usage: make bench-scenario SCENARIO=reeve|electron|chrome-tabs" && exit 1)
	$(BENCH) --scenario $(SCENARIO) --duration $(BENCH_DURATION) --out $(BENCH_OUT)

# Convenience shorthands
bench-reeve: release
	$(BENCH) --scenario reeve --duration $(BENCH_DURATION) --out $(BENCH_OUT)

bench-electron: release
	$(BENCH) --scenario electron --duration $(BENCH_DURATION) --out $(BENCH_OUT)

bench-chrome: release
	$(BENCH) --scenario chrome-tabs --duration $(BENCH_DURATION) --out $(BENCH_OUT)

# Quick smoke test: 60 s per scenario, ~3 min total
bench-quick: release
	$(BENCH) --scenario all --duration 60 --out $(BENCH_OUT)

# Publication run: 10 min per scenario, ~30 min total
bench-long: release
	$(BENCH) --scenario all --duration 600 --out $(BENCH_OUT)

# ---------------------------------------------------------------------------
# Clean
# ---------------------------------------------------------------------------

clean:
	$(CARGO) clean

distclean: clean
	rm -rf $(BENCH_OUT)