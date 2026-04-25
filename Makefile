.PHONY: build test run check check-scripts setup-hooks setup-large-corpus ensure-cache test-large-corpus large-corpus-report large-corpus-report-from-log large-corpus-report-open test-oracle-shfmt test-oracle-shfmt-fixtures test-oracle-shfmt-benchmark test-oracle-shellcheck-cli fuzz-setup fuzz-list fuzz-smoke fuzz-run fuzz-cli bench bench-save bench-compare bench-parser bench-arithmetic bench-lexer bench-semantic bench-linter bench-formatter bench-large-corpus-hotspots bench-macro bench-macro-single bench-macro-format bench-macro-format-summary bench-macro-format-single bench-macro-site-local profile-parser profile-parser-view profile-arithmetic profile-arithmetic-view profile-formatter profile-formatter-view profile-linter profile-linter-view profile-cli profile-cli-view profile-large-corpus profile-large-corpus-view flame-parser flame-arithmetic flame-formatter flame-linter flame-cli harden-release check-release-security

ARGS ?= --help
BENCH_FILE ?=
NIX_DEVELOP ?= nix --extra-experimental-features 'nix-command flakes' develop --command
UV_PYTHON ?= uv run python
FUZZ_SANITIZER_ARG ?= $(if $(filter Darwin,$(shell uname -s)),,-s none)
FUZZ_SMOKE_ARGS ?= -runs=1
FUZZ_TARGET ?= parser_fuzz
FUZZ_ARGS ?= -max_total_time=60
FUZZ_CLI_ARGS ?= --dialect sh --profile smoke --count 1 --seed 0
FUZZ_CARGO_ENV ?= export PATH="$$HOME/.cargo/bin:$$PATH"; . "$$HOME/.cargo/env" >/dev/null 2>&1 || true;
PROFILE_CASE ?= nvm
PROFILE_FILE ?= crates/shuck-benchmark/resources/files/$(PROFILE_CASE).sh
PROFILE_DIR ?= .cache/profiles
PROFILE_RATE ?= 1000
PROFILE_ITERATIONS ?= 1
PROFILE_CORPUS_FIXTURE ?= xwmx__nb__nb
PROFILE_CORPUS_ITERATIONS ?= 1
SHUCK_LARGE_CORPUS_TIMEOUT_SECS ?= 300
SHUCK_LARGE_CORPUS_SHUCK_TIMEOUT_SECS ?=
SHUCK_LARGE_CORPUS_SAMPLE_PERCENT ?= 100
SHUCK_LARGE_CORPUS_MAPPED_ONLY ?= 1
SHUCK_LARGE_CORPUS_KEEP_GOING ?= 1
SHUCK_LARGE_CORPUS_TIMING ?= 0
SHUCK_LARGE_CORPUS_RULES ?=
LARGE_CORPUS_REPORT_DIR ?= target/large-corpus-report
LARGE_CORPUS_REPORT_LOG ?= $(LARGE_CORPUS_REPORT_DIR)/latest.log
LARGE_CORPUS_REPORT_HTML ?= $(LARGE_CORPUS_REPORT_DIR)/index.html
BENCHMARK_WEBSITE_LOCAL_OUTPUT ?= website/generated/benchmarks/local-m5-max.json
BENCHMARK_WEBSITE_BENCH_DIR ?= $(or $(SHUCK_BENCHMARK_OUTPUT_DIR),.cache)
LARGE_CORPUS_SHUCK_TIMEOUT_ENV := $(if $(strip $(SHUCK_LARGE_CORPUS_SHUCK_TIMEOUT_SECS)),SHUCK_LARGE_CORPUS_SHUCK_TIMEOUT_SECS=$(SHUCK_LARGE_CORPUS_SHUCK_TIMEOUT_SECS))

setup-hooks:
	git config core.hooksPath .githooks

build:
	cargo build

test:
	cargo test

setup-large-corpus:
	./scripts/corpus-download.sh

fuzz-setup:
	bash ./scripts/fuzz-init.sh

fuzz-list:
	bash ./scripts/fuzz-init.sh --ci
	bash -lc '$(FUZZ_CARGO_ENV) cd fuzz && cargo fuzz list'

fuzz-smoke:
	bash ./scripts/fuzz-init.sh --ci
	bash -lc '$(FUZZ_CARGO_ENV) cd fuzz && cargo +nightly fuzz run $(FUZZ_SANITIZER_ARG) parser_fuzz -- $(FUZZ_SMOKE_ARGS)'
	bash -lc '$(FUZZ_CARGO_ENV) cd fuzz && cargo +nightly fuzz run $(FUZZ_SANITIZER_ARG) recovered_parser_fuzz -- $(FUZZ_SMOKE_ARGS)'
	bash -lc '$(FUZZ_CARGO_ENV) cd fuzz && cargo +nightly fuzz run $(FUZZ_SANITIZER_ARG) formatter_consistency_fuzz -- $(FUZZ_SMOKE_ARGS)'
	bash -lc '$(FUZZ_CARGO_ENV) cd fuzz && cargo +nightly fuzz run $(FUZZ_SANITIZER_ARG) linter_no_panic_fuzz -- $(FUZZ_SMOKE_ARGS)'

fuzz-run:
	test -n "$(FUZZ_TARGET)"
	bash ./scripts/fuzz-init.sh --ci
	bash -lc '$(FUZZ_CARGO_ENV) cd fuzz && cargo +nightly fuzz run $(FUZZ_SANITIZER_ARG) "$(FUZZ_TARGET)" -- $(FUZZ_ARGS)'

fuzz-cli:
	cargo build -p shuck-cli
	python3 ./scripts/fuzz_cli.py --shuck-bin ./target/debug/shuck $(FUZZ_CLI_ARGS)

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
	@case "$(SHUCK_LARGE_CORPUS_TIMING)" in \
		1|true|TRUE|yes|YES|on|ON) \
			SHUCK_TEST_LARGE_CORPUS=1 \
			SHUCK_LARGE_CORPUS_TIMEOUT_SECS=$(SHUCK_LARGE_CORPUS_TIMEOUT_SECS) \
			$(LARGE_CORPUS_SHUCK_TIMEOUT_ENV) \
			SHUCK_LARGE_CORPUS_RULES=$(SHUCK_LARGE_CORPUS_RULES) \
			SHUCK_LARGE_CORPUS_SAMPLE_PERCENT=$(SHUCK_LARGE_CORPUS_SAMPLE_PERCENT) \
			SHUCK_LARGE_CORPUS_MAPPED_ONLY=$(SHUCK_LARGE_CORPUS_MAPPED_ONLY) \
			SHUCK_LARGE_CORPUS_KEEP_GOING=$(SHUCK_LARGE_CORPUS_KEEP_GOING) \
			SHUCK_LARGE_CORPUS_TIMING=$(SHUCK_LARGE_CORPUS_TIMING) \
			$(NIX_DEVELOP) cargo test -p shuck-cli --test large_corpus large_corpus_conforms_with_shellcheck -- --ignored --exact --nocapture ;; \
		*) \
			SHUCK_TEST_LARGE_CORPUS=1 \
			SHUCK_LARGE_CORPUS_TIMEOUT_SECS=$(SHUCK_LARGE_CORPUS_TIMEOUT_SECS) \
			$(LARGE_CORPUS_SHUCK_TIMEOUT_ENV) \
			SHUCK_LARGE_CORPUS_RULES=$(SHUCK_LARGE_CORPUS_RULES) \
			SHUCK_LARGE_CORPUS_SAMPLE_PERCENT=$(SHUCK_LARGE_CORPUS_SAMPLE_PERCENT) \
			SHUCK_LARGE_CORPUS_MAPPED_ONLY=$(SHUCK_LARGE_CORPUS_MAPPED_ONLY) \
			SHUCK_LARGE_CORPUS_KEEP_GOING=$(SHUCK_LARGE_CORPUS_KEEP_GOING) \
			SHUCK_LARGE_CORPUS_TIMING=$(SHUCK_LARGE_CORPUS_TIMING) \
			$(NIX_DEVELOP) cargo test -p shuck-cli --test large_corpus -- --ignored --nocapture ;; \
	esac

test-large-corpus-zsh: ensure-cache
	SHUCK_TEST_LARGE_CORPUS=1 \
	SHUCK_LARGE_CORPUS_TIMEOUT_SECS=$(SHUCK_LARGE_CORPUS_TIMEOUT_SECS) \
	$(LARGE_CORPUS_SHUCK_TIMEOUT_ENV) \
	SHUCK_LARGE_CORPUS_SAMPLE_PERCENT=$(SHUCK_LARGE_CORPUS_SAMPLE_PERCENT) \
	SHUCK_LARGE_CORPUS_KEEP_GOING=$(SHUCK_LARGE_CORPUS_KEEP_GOING) \
	$(NIX_DEVELOP) cargo test -p shuck-cli --test large_corpus large_corpus_zsh_fixtures_parse -- --ignored --exact --nocapture

large-corpus-report-from-log:
	test -f "$(LARGE_CORPUS_REPORT_LOG)"
	$(UV_PYTHON) ./scripts/large_corpus_report.py --log "$(LARGE_CORPUS_REPORT_LOG)" --output "$(LARGE_CORPUS_REPORT_HTML)"
	@echo "large corpus HTML report: $$(cd . && pwd)/$(LARGE_CORPUS_REPORT_HTML)"

large-corpus-report:
	@case "$(SHUCK_LARGE_CORPUS_TIMING)" in \
		1|true|TRUE|yes|YES|on|ON) \
			echo "large-corpus-report does not support SHUCK_LARGE_CORPUS_TIMING=1; run 'make test-large-corpus SHUCK_LARGE_CORPUS_TIMING=1' directly."; \
			exit 1 ;; \
	esac
	@$(MAKE) --no-print-directory ensure-cache
	@mkdir -p "$(LARGE_CORPUS_REPORT_DIR)"
	@status=0; \
	$(MAKE) --no-print-directory test-large-corpus >"$(LARGE_CORPUS_REPORT_LOG)" 2>&1 || status=$$?; \
	$(UV_PYTHON) ./scripts/compact_large_corpus_log.py <"$(LARGE_CORPUS_REPORT_LOG)"; \
	$(UV_PYTHON) ./scripts/large_corpus_report.py --log "$(LARGE_CORPUS_REPORT_LOG)" --output "$(LARGE_CORPUS_REPORT_HTML)"; \
	echo "large corpus HTML report: $$(cd . && pwd)/$(LARGE_CORPUS_REPORT_HTML)"; \
	if [ $$status -ne 0 ]; then \
		echo "large corpus run exited with $$status; the HTML report was still generated from the captured log"; \
	fi

large-corpus-report-open: large-corpus-report
	$(UV_PYTHON) -m webbrowser "file://$$(cd . && pwd)/$(LARGE_CORPUS_REPORT_HTML)"

test-oracle-shfmt: test-oracle-shfmt-fixtures test-oracle-shfmt-benchmark

test-oracle-shfmt-fixtures:
	SHUCK_RUN_SHFMT_ORACLE=1 \
	$(NIX_DEVELOP) cargo test -p shuck-formatter --test oracle_shfmt selected_fixtures_match_shfmt -- --ignored --exact --nocapture

test-oracle-shfmt-benchmark:
	SHUCK_RUN_SHFMT_ORACLE=1 \
	$(NIX_DEVELOP) cargo test -p shuck-formatter --test oracle_shfmt benchmark_corpus_matches_shfmt -- --ignored --exact --nocapture

test-oracle-shellcheck-cli:
	$(NIX_DEVELOP) cargo test -p shuck-cli --test oracle_shellcheck_cli -- --ignored --nocapture

run:
	cargo run -p shuck-cli -- $(ARGS)

bench:
	cargo bench -p shuck-benchmark

bench-save:
	python3 scripts/benchmarks/run_criterion.py --repo-root . --save-baseline main

bench-compare:
	python3 scripts/benchmarks/run_criterion.py --repo-root . --baseline main

bench-memory-save:
	python3 scripts/benchmarks/run_parser_memory.py --repo-root . --save-baseline main --release

bench-memory-compare:
	python3 scripts/benchmarks/run_parser_memory.py --repo-root . --baseline main --release

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

bench-formatter:
	cargo bench -p shuck-benchmark --bench formatter

bench-large-corpus-hotspots: ensure-cache
	cargo bench -p shuck-benchmark --features large-corpus-hotspots --bench large_corpus_hotspots

bench-macro:
	$(NIX_DEVELOP) ./scripts/benchmarks/setup.sh hyperfine shellcheck
	$(NIX_DEVELOP) ./scripts/benchmarks/run.sh

bench-macro-site-local: bench-macro
	$(NIX_DEVELOP) python3 ./scripts/benchmarks/export_website_data.py --repo-root . --bench-dir "$(BENCHMARK_WEBSITE_BENCH_DIR)" --output "$(BENCHMARK_WEBSITE_LOCAL_OUTPUT)" --dataset-id local-m5-max --dataset-name "Apple M5 Max checked-in snapshot" --dataset-description "Checked-in make bench-macro results captured on an Apple M5 Max macOS development machine." --environment-kind local --environment-label "Apple M5 Max macOS snapshot" --notes "Regenerate this checked-in snapshot on the Apple M5 Max machine when you want to refresh the website's local reference numbers."

bench-macro-single:
	test -n "$(BENCH_FILE)"
	$(NIX_DEVELOP) ./scripts/benchmarks/setup.sh hyperfine shellcheck
	$(NIX_DEVELOP) ./scripts/benchmarks/run_single.sh "$(BENCH_FILE)"

bench-macro-format:
	$(NIX_DEVELOP) ./scripts/benchmarks/setup.sh hyperfine shfmt
	$(NIX_DEVELOP) ./scripts/benchmarks/run_formatter.sh
	$(NIX_DEVELOP) ./scripts/benchmarks/summarize_formatter.sh

bench-macro-format-summary:
	$(NIX_DEVELOP) ./scripts/benchmarks/summarize_formatter.sh

bench-macro-format-single:
	test -n "$(BENCH_FILE)"
	$(NIX_DEVELOP) ./scripts/benchmarks/setup.sh hyperfine shfmt
	$(NIX_DEVELOP) ./scripts/benchmarks/run_formatter_single.sh "$(BENCH_FILE)"

profile-parser:
	$(NIX_DEVELOP) ./scripts/profiling/profile_bench.sh parser "$(PROFILE_CASE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-parser-view:
	SAMPLY_VIEW=1 $(NIX_DEVELOP) ./scripts/profiling/profile_bench.sh parser "$(PROFILE_CASE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-arithmetic:
	$(NIX_DEVELOP) ./scripts/profiling/profile_bench.sh arithmetic "$(PROFILE_CASE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-arithmetic-view:
	SAMPLY_VIEW=1 $(NIX_DEVELOP) ./scripts/profiling/profile_bench.sh arithmetic "$(PROFILE_CASE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-formatter:
	$(NIX_DEVELOP) ./scripts/profiling/profile_bench.sh formatter "$(PROFILE_CASE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-formatter-view:
	SAMPLY_VIEW=1 $(NIX_DEVELOP) ./scripts/profiling/profile_bench.sh formatter "$(PROFILE_CASE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-linter:
	$(NIX_DEVELOP) ./scripts/profiling/profile_bench.sh linter "$(PROFILE_CASE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-linter-view:
	SAMPLY_VIEW=1 $(NIX_DEVELOP) ./scripts/profiling/profile_bench.sh linter "$(PROFILE_CASE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-cli:
	$(NIX_DEVELOP) ./scripts/profiling/profile_cli.sh "$(PROFILE_FILE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-cli-view:
	SAMPLY_VIEW=1 $(NIX_DEVELOP) ./scripts/profiling/profile_cli.sh "$(PROFILE_FILE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)"

profile-large-corpus: ensure-cache
	$(NIX_DEVELOP) ./scripts/profiling/profile_large_corpus.sh "$(PROFILE_CORPUS_FIXTURE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)" "$(PROFILE_CORPUS_ITERATIONS)"

profile-large-corpus-view: ensure-cache
	SAMPLY_VIEW=1 $(NIX_DEVELOP) ./scripts/profiling/profile_large_corpus.sh "$(PROFILE_CORPUS_FIXTURE)" "$(PROFILE_DIR)" "$(PROFILE_RATE)" "$(PROFILE_ITERATIONS)" "$(PROFILE_CORPUS_ITERATIONS)"

flame-parser:
	@mkdir -p $(PROFILE_DIR)
	cargo flamegraph --profile profiling -p shuck-benchmark --bench parser -o $(PROFILE_DIR)/flame-parser-$(PROFILE_CASE).svg -- --bench $(PROFILE_CASE) --noplot
	open $(PROFILE_DIR)/flame-parser-$(PROFILE_CASE).svg

flame-arithmetic:
	@mkdir -p $(PROFILE_DIR)
	cargo flamegraph --profile profiling -p shuck-benchmark --bench arithmetic -o $(PROFILE_DIR)/flame-arithmetic-$(PROFILE_CASE).svg -- --bench $(PROFILE_CASE) --noplot
	open $(PROFILE_DIR)/flame-arithmetic-$(PROFILE_CASE).svg

flame-formatter:
	@mkdir -p $(PROFILE_DIR)
	cargo flamegraph --profile profiling -p shuck-benchmark --bench formatter -o $(PROFILE_DIR)/flame-formatter-$(PROFILE_CASE).svg -- --bench $(PROFILE_CASE) --noplot
	open $(PROFILE_DIR)/flame-formatter-$(PROFILE_CASE).svg

flame-linter:
	@mkdir -p $(PROFILE_DIR)
	cargo flamegraph --profile profiling -p shuck-benchmark --bench linter -o $(PROFILE_DIR)/flame-linter-$(PROFILE_CASE).svg -- --bench $(PROFILE_CASE) --noplot
	open $(PROFILE_DIR)/flame-linter-$(PROFILE_CASE).svg

flame-cli:
	@mkdir -p $(PROFILE_DIR)
	cargo flamegraph --profile profiling -p shuck-cli -o $(PROFILE_DIR)/flame-cli.svg -- check --no-cache "$(PROFILE_FILE)"
	open $(PROFILE_DIR)/flame-cli.svg

harden-release:
	python3 scripts/check-release-security.py fix

check-release-security:
	python3 scripts/check-release-security.py check

check-scripts:
	cargo run -q -p shuck-cli -- check --no-cache scripts
check:
	cargo fmt -- --check
	cargo clippy --all-targets -- -D warnings
	$(NIX_DEVELOP) env RUSTC_BOOTSTRAP=1 cargo udeps --all-targets
	$(MAKE) --no-print-directory check-scripts
