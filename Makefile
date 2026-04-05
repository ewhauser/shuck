.PHONY: build test run check corpus-download corpus-extract test-corpus setup-large-corpus test-large-corpus bench bench-save bench-compare bench-parser bench-lexer bench-linter bench-macro bench-macro-single profile-parser profile-parser-view profile-linter profile-linter-view profile-cli profile-cli-view flame-parser flame-linter flame-cli

ARGS ?= --help
CORPUS_TAG ?= v0.0.0-test-files
CORPUS_REPO ?= ewhauser/shuck-rs
CORPUS_URL ?= https://github.com/ewhauser/shuck-rs/releases/download/$(CORPUS_TAG)/corpus.tar.gz
CORPUS_DIR ?= .cache/scripts
CORPUS_ARCHIVE ?= $(CORPUS_DIR)/corpus.tar.gz
BENCH_FILE ?=
NIX_DEVELOP ?= nix --extra-experimental-features 'nix-command flakes' develop --command
PROFILE_CASE ?= nvm
PROFILE_FILE ?= crates/shuck-benchmark/resources/files/$(PROFILE_CASE).sh
PROFILE_DIR ?= .cache/profiles
PROFILE_RATE ?= 1000
PROFILE_ITERATIONS ?= 1

build:
	cargo build -p shuck -p shuck-cache

test:
	cargo test -p shuck -p shuck-cache

corpus-download:
	mkdir -p $(CORPUS_DIR)
	if [ ! -f "$(CORPUS_ARCHIVE)" ]; then \
		if command -v gh >/dev/null 2>&1; then \
			gh release download $(CORPUS_TAG) --repo $(CORPUS_REPO) --pattern corpus.tar.gz --dir $(CORPUS_DIR); \
		else \
			curl -L --fail --show-error --silent "$(CORPUS_URL)" -o "$(CORPUS_ARCHIVE)"; \
		fi; \
	fi

corpus-extract: corpus-download
	if [ -z "$$(find "$(CORPUS_DIR)" -maxdepth 1 -type f -name '*.json' -print -quit)" ]; then \
		tar -xzf "$(CORPUS_ARCHIVE)" -C "$(CORPUS_DIR)"; \
	fi

test-corpus: corpus-extract
	SHUCK_AST_CORPUS_DIR="$$(pwd)/$(CORPUS_DIR)" cargo test -p shuck-ast-printer --test corpus -- --ignored

setup-large-corpus:
	./scripts/corpus-download.sh

test-large-corpus:
	SHUCK_TEST_LARGE_CORPUS=1 cargo test -p shuck --test large_corpus -- --ignored

run:
	cargo run -p shuck -- $(ARGS)

bench:
	cargo bench -p shuck-benchmark

bench-save:
	cargo bench -p shuck-benchmark -- --save-baseline=main

bench-compare:
	cargo bench -p shuck-benchmark -- --baseline=main

bench-parser:
	cargo bench -p shuck-benchmark --bench parser

bench-lexer:
	cargo bench -p shuck-benchmark --bench lexer

bench-linter:
	cargo bench -p shuck-benchmark --bench linter

bench-macro:
	$(NIX_DEVELOP) ./scripts/benchmarks/setup.sh
	$(NIX_DEVELOP) ./scripts/benchmarks/run.sh

bench-macro-single:
	test -n "$(BENCH_FILE)"
	$(NIX_DEVELOP) ./scripts/benchmarks/setup.sh
	$(NIX_DEVELOP) ./scripts/benchmarks/run_single.sh "$(BENCH_FILE)"

profile-parser:
	$(NIX_DEVELOP) ./scripts/profiling/profile_bench.sh parser "$(PROFILE_CASE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-parser-view:
	SAMPLY_VIEW=1 $(NIX_DEVELOP) ./scripts/profiling/profile_bench.sh parser "$(PROFILE_CASE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-linter:
	$(NIX_DEVELOP) ./scripts/profiling/profile_bench.sh linter "$(PROFILE_CASE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-linter-view:
	SAMPLY_VIEW=1 $(NIX_DEVELOP) ./scripts/profiling/profile_bench.sh linter "$(PROFILE_CASE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-cli:
	$(NIX_DEVELOP) ./scripts/profiling/profile_cli.sh "$(PROFILE_FILE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-cli-view:
	SAMPLY_VIEW=1 $(NIX_DEVELOP) ./scripts/profiling/profile_cli.sh "$(PROFILE_FILE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

flame-parser:
	@mkdir -p $(PROFILE_DIR)
	cargo flamegraph --profile profiling -p shuck-benchmark --bench parser -o $(PROFILE_DIR)/flame-parser-$(PROFILE_CASE).svg -- $(PROFILE_CASE) --noplot
	open $(PROFILE_DIR)/flame-parser-$(PROFILE_CASE).svg

flame-linter:
	@mkdir -p $(PROFILE_DIR)
	cargo flamegraph --profile profiling -p shuck-benchmark --bench linter -o $(PROFILE_DIR)/flame-linter-$(PROFILE_CASE).svg -- $(PROFILE_CASE) --noplot
	open $(PROFILE_DIR)/flame-linter-$(PROFILE_CASE).svg

flame-cli:
	@mkdir -p $(PROFILE_DIR)
	cargo flamegraph --profile profiling -p shuck -o $(PROFILE_DIR)/flame-cli.svg -- check --no-cache "$(PROFILE_FILE)"
	open $(PROFILE_DIR)/flame-cli.svg

check:
	cargo fmt -- --check
	cargo clippy --all-targets -- -D warnings
	cargo +nightly udeps --all-targets
