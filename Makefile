CARGO ?= cargo

.PHONY: all build run check test fmt clippy release clean distclean \
        bench bench-scenario bench-reeve bench-electron bench-chrome \
        bench-quick bench-long

all: build

build:
	$(CARGO) build

release:
	$(CARGO) build --release

run:
	RUST_LOG=info,reeve=debug $(CARGO) run

check:
	$(CARGO) check

test:
	$(CARGO) test

fmt:
	$(CARGO) fmt

clippy:
	$(CARGO) clippy -- -D warnings

BENCH           := $(CARGO) run --release --bin benchmark --
BENCH_DURATION  ?= 120
BENCH_OUT       ?= benchmark-results

bench: release
	$(BENCH) --scenario all --duration $(BENCH_DURATION) --out $(BENCH_OUT)

bench-scenario: release
	@test -n "$(SCENARIO)" || (echo "Usage: make bench-scenario SCENARIO=reeve|electron|chrome-tabs" && exit 1)
	$(BENCH) --scenario $(SCENARIO) --duration $(BENCH_DURATION) --out $(BENCH_OUT)

bench-reeve: release
	$(BENCH) --scenario reeve --duration $(BENCH_DURATION) --out $(BENCH_OUT)

bench-electron: release
	$(BENCH) --scenario electron --duration $(BENCH_DURATION) --out $(BENCH_OUT)

bench-chrome: release
	$(BENCH) --scenario chrome-tabs --duration $(BENCH_DURATION) --out $(BENCH_OUT)

bench-quick: release
	$(BENCH) --scenario all --duration 60 --out $(BENCH_OUT)

bench-long: release
	$(BENCH) --scenario all --duration 600 --out $(BENCH_OUT)

clean:
	$(CARGO) clean

distclean: clean
	rm -rf $(BENCH_OUT)