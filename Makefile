.PHONY: build test run check setup-hooks setup-large-corpus ensure-cache test-large-corpus bench bench-save bench-compare bench-parser bench-arithmetic bench-lexer bench-linter bench-macro bench-macro-single profile-parser profile-parser-view profile-linter profile-linter-view profile-cli profile-cli-view flame-parser flame-linter flame-cli

ARGS ?= --help
BENCH_FILE ?=
NIX_DEVELOP ?= nix --extra-experimental-features 'nix-command flakes' develop --command
PROFILE_CASE ?= nvm
PROFILE_FILE ?= crates/shuck-benchmark/resources/files/$(PROFILE_CASE).sh
PROFILE_DIR ?= .cache/profiles
PROFILE_RATE ?= 1000
PROFILE_ITERATIONS ?= 1
SHUCK_LARGE_CORPUS_TIMEOUT_SECS ?= 300
SHUCK_LARGE_CORPUS_SHUCK_TIMEOUT_SECS ?= 30
SHUCK_LARGE_CORPUS_SAMPLE_PERCENT ?= 100
SHUCK_LARGE_CORPUS_MAPPED_ONLY ?= 1
SHUCK_LARGE_CORPUS_KEEP_GOING ?= 1
SHUCK_LARGE_CORPUS_RULES ?=

setup-hooks:
	git config core.hooksPath .githooks

build:
	cargo build

test:
	cargo test

setup-large-corpus:
	./scripts/corpus-download.sh

ensure-cache:
	@if [ ! -e .cache ]; then \
		main_worktree=$$(git worktree list --porcelain | head -1 | sed 's/^worktree //'); \
		if [ "$$main_worktree" != "$$(pwd)" ] && [ -d "$$main_worktree/.cache" ]; then \
			echo "Symlinking .cache -> $$main_worktree/.cache"; \
			ln -s "$$main_worktree/.cache" .cache; \
		else \
			echo "No .cache found in main worktree ($$main_worktree). Run 'make setup-large-corpus' first."; \
			exit 1; \
		fi \
	fi

test-large-corpus: ensure-cache
	SHUCK_TEST_LARGE_CORPUS=1 \
	SHUCK_LARGE_CORPUS_TIMEOUT_SECS=$(SHUCK_LARGE_CORPUS_TIMEOUT_SECS) \
	SHUCK_LARGE_CORPUS_SHUCK_TIMEOUT_SECS=$(SHUCK_LARGE_CORPUS_SHUCK_TIMEOUT_SECS) \
	SHUCK_LARGE_CORPUS_RULES=$(SHUCK_LARGE_CORPUS_RULES) \
	SHUCK_LARGE_CORPUS_SAMPLE_PERCENT=$(SHUCK_LARGE_CORPUS_SAMPLE_PERCENT) \
	SHUCK_LARGE_CORPUS_MAPPED_ONLY=$(SHUCK_LARGE_CORPUS_MAPPED_ONLY) \
	SHUCK_LARGE_CORPUS_KEEP_GOING=$(SHUCK_LARGE_CORPUS_KEEP_GOING) \
	$(NIX_DEVELOP) cargo test -p shuck --test large_corpus -- --ignored --nocapture

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

bench-arithmetic:
	cargo bench -p shuck-benchmark --bench arithmetic

bench-lexer:
	cargo bench -p shuck-benchmark --bench lexer

bench-semantic:
	cargo bench -p shuck-benchmark --bench semantic

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
	$(NIX_DEVELOP) cargo +nightly udeps --all-targets
