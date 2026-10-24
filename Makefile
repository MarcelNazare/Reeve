CARGO ?= cargo
BIN   := reeve

.PHONY: all build run check clean test release fmt clippy

all: build

build:
	$(CARGO) build

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

release:
	$(CARGO) build --release

clean:
	$(CARGO) clean