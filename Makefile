.PHONY: build test run check corpus-download corpus-extract test-corpus setup-large-corpus test-large-corpus

ARGS ?= --help
CORPUS_TAG ?= v0.0.0-test-files
CORPUS_REPO ?= ewhauser/shuck-rs
CORPUS_URL ?= https://github.com/ewhauser/shuck-rs/releases/download/$(CORPUS_TAG)/corpus.tar.gz
CORPUS_DIR ?= .cache/scripts
CORPUS_ARCHIVE ?= $(CORPUS_DIR)/corpus.tar.gz

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

check:
	cargo fmt -- --check
	cargo clippy --all-targets -- -D warnings
	cargo +nightly udeps --all-targets
