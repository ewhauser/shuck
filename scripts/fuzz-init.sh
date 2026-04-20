#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
FUZZ_DIR="${ROOT_DIR}/fuzz"
CORPUS_DIR="${FUZZ_DIR}/corpus"
COMMON_CORPUS_DIR="${CORPUS_DIR}/common"
FORMATTER_CORPUS_DIR="${CORPUS_DIR}/formatter"
ARTIFACTS_DIR="${FUZZ_DIR}/artifacts"

COMMON_TARGETS=(
  parser_fuzz
  lexer_fuzz
  arithmetic_fuzz
  glob_fuzz
  recovered_parser_fuzz
  linter_no_panic_fuzz
)

FORMATTER_TARGETS=(
  formatter_consistency_fuzz
  formatter_validity_fuzz
)

CI_MODE=0
RUN_CMIN=0
USE_LARGE_CORPUS=0

usage() {
  cat <<'EOF'
Usage: bash ./scripts/fuzz-init.sh [--ci] [--cmin] [--large-corpus]

  --ci            Non-interactive setup for CI and automation
  --cmin          Run cargo-fuzz corpus minimization after seeding
  --large-corpus  Copy shell-like fixtures from .cache/large-corpus when present
EOF
}

while (($#)); do
  case "$1" in
    --ci)
      CI_MODE=1
      ;;
    --cmin)
      RUN_CMIN=1
      ;;
    --large-corpus)
      USE_LARGE_CORPUS=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
  shift
done

ensure_rustup() {
  export PATH="${HOME}/.cargo/bin:${PATH}"

  if command -v rustup >/dev/null 2>&1; then
    return 0
  fi

  if [[ -r "${HOME}/.cargo/env" ]]; then
    # shellcheck disable=SC1091
    source "${HOME}/.cargo/env"
  fi

  if command -v rustup >/dev/null 2>&1; then
    return 0
  fi

  if ! command -v curl >/dev/null 2>&1; then
    echo "rustup is required for fuzzing, and curl is not available to install it." >&2
    echo "Install rustup manually, then rerun this script." >&2
    exit 1
  fi

  echo "Installing rustup for fuzzing support..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path

  export PATH="${HOME}/.cargo/bin:${PATH}"
  if [[ -r "${HOME}/.cargo/env" ]]; then
    # shellcheck disable=SC1091
    source "${HOME}/.cargo/env"
  fi

  if ! command -v rustup >/dev/null 2>&1; then
    echo "rustup installation completed, but rustup is still unavailable on PATH." >&2
    exit 1
  fi
}

ensure_toolchain() {
  ensure_rustup

  if ! rustup toolchain list | grep -q '^nightly'; then
    echo "Installing nightly Rust toolchain..."
    rustup toolchain install nightly --profile minimal
  fi

  if ! cargo fuzz --help >/dev/null 2>&1; then
    echo "Installing cargo-fuzz..."
    cargo install cargo-fuzz --locked
  fi
}

ensure_layout() {
  mkdir -p "${COMMON_CORPUS_DIR}" "${FORMATTER_CORPUS_DIR}" "${ARTIFACTS_DIR}"
  find "${COMMON_CORPUS_DIR}" -mindepth 1 -maxdepth 1 -type f -delete
  find "${FORMATTER_CORPUS_DIR}" -mindepth 1 -maxdepth 1 -type f -delete
  (
    cd "${CORPUS_DIR}"
    for target in "${COMMON_TARGETS[@]}"; do
      ln -snf "common" "${target}"
    done
    for target in "${FORMATTER_TARGETS[@]}"; do
      ln -snf "formatter" "${target}"
    done
  )
}

seed_from_directory() {
  local destination="$1"
  local directory="$2"
  [[ -d "${directory}" ]] || return 0

  while IFS= read -r file; do
    local relative_path="${file:$(( ${#ROOT_DIR} + 1 ))}"
    local sanitized_name="${relative_path//\//__}"
    cp "${file}" "${destination}/${sanitized_name}"
  done < <(
    find "${directory}" -type f \
      \( -name '*.sh' -o -name '*.bash' -o -name '*.dash' -o -name '*.ksh' -o -name '*.mksh' -o -name '*.zsh' \) \
      | sort
  )
}

maybe_seed_large_corpus() {
  local corpus_root="${ROOT_DIR}/.cache/large-corpus"
  [[ "${USE_LARGE_CORPUS}" -eq 1 ]] || return 0
  [[ -d "${corpus_root}" ]] || return 0

  echo "Seeding from .cache/large-corpus..."
  seed_from_directory "${COMMON_CORPUS_DIR}" "${corpus_root}"
}

seed_repo_corpus() {
  echo "Seeding fuzz corpus from repository fixtures..."
  seed_from_directory "${COMMON_CORPUS_DIR}" "${ROOT_DIR}/crates/shuck-linter/resources/test/fixtures"
  seed_from_directory "${COMMON_CORPUS_DIR}" "${ROOT_DIR}/crates/shuck-formatter/tests/oracle-fixtures"
  seed_from_directory "${COMMON_CORPUS_DIR}" "${ROOT_DIR}/crates/shuck-benchmark/resources/files"
  seed_from_directory "${COMMON_CORPUS_DIR}" "${ROOT_DIR}/scripts"
  seed_from_directory "${FORMATTER_CORPUS_DIR}" "${ROOT_DIR}/crates/shuck-formatter/tests/oracle-fixtures"

  if [[ "${CI_MODE}" -eq 0 && -d "${ROOT_DIR}/.cache/large-corpus" && "${USE_LARGE_CORPUS}" -eq 0 ]]; then
    read -r -p "Copy shell fixtures from .cache/large-corpus too? [y/N] " reply
    if [[ "${reply}" =~ ^[Yy]$ ]]; then
      USE_LARGE_CORPUS=1
    fi
  fi

  maybe_seed_large_corpus
}

run_cmin() {
  [[ "${RUN_CMIN}" -eq 1 ]] || return 0

  echo "Running corpus minimization..."
  (
    cd "${FUZZ_DIR}"
    if [[ "$(uname -s)" == "Darwin" ]]; then
      cargo +nightly fuzz cmin parser_fuzz corpus/common -- -timeout=5
    else
      cargo +nightly fuzz cmin -s none parser_fuzz corpus/common -- -timeout=5
    fi
  )
}

ensure_toolchain
ensure_layout
seed_repo_corpus
run_cmin

echo "Fuzz setup complete."
