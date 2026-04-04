.PHONY: build test run check

ARGS ?= --help

build:
	cargo build -p shuck -p shuck-cache

test:
	cargo test -p shuck -p shuck-cache

run:
	cargo run -p shuck -- $(ARGS)

check:
	cargo fmt -- --check
	cargo clippy --all-targets -- -D warnings
	cargo +nightly udeps --all-targets
