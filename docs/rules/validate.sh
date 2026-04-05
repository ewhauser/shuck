#!/usr/bin/env bash

set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd "${script_dir}/../.." && pwd)
shell_checks_root="${repo_root}/../shell-checks"

shopt -s nullglob

check_yq() {
  local file=$1
  local expr=$2
  local message=$3

  if ! yq -e "${expr}" "${file}" >/dev/null; then
    printf 'ERROR %s: %s\n' "$(basename "${file}")" "${message}" >&2
    return 1
  fi
}

resolve_shell_checks_path() {
  local path=$1
  path=${path#shell-checks/}
  printf '%s/%s' "${shell_checks_root}" "${path}"
}

normalized_list() {
  local file=$1
  local expr=$2

  yq -r "${expr}[]" "${file}" | LC_ALL=C sort
}

check_unique_field() {
  local field=$1
  local label=$2
  shift 2

  local tmp
  local file
  local basename
  local value
  local failed=0

  tmp=$(mktemp)
  trap 'rm -f "${tmp}"' RETURN

  for file in "$@"; do
    basename=$(basename "${file}")
    value=$(yq -r "${field}" "${file}")

    if [[ -z "${value}" || "${value}" == "null" ]]; then
      printf 'ERROR %s: %s is missing, so uniqueness could not be checked\n' "${basename}" "${label}" >&2
      failed=1
      continue
    fi

    printf '%s\t%s\n' "${value}" "${basename}" >> "${tmp}"
  done

  while IFS=$'\t' read -r value basenames; do
    [[ -n "${value}" ]] || continue
    printf 'ERROR duplicate %s %s in %s\n' "${label}" "${value}" "${basenames}" >&2
    failed=1
  done < <(
    sort -k1,1 "${tmp}" |
      awk -F '\t' '
        {
          if ($1 == current) {
            files = files ", " $2
          } else {
            if (count > 1) {
              print current "\t" files
            }
            current = $1
            files = $2
            count = 1
            next
          }
          count++
        }
        END {
          if (count > 1) {
            print current "\t" files
          }
        }
      '
  )

  rm -f "${tmp}"
  trap - RETURN

  return "${failed}"
}

validate_file() {
  local file=$1
  local basename stem legacy_code rule_path example_path doc_shells source_shells failed=0

  basename=$(basename "${file}")
  stem=${basename%.yaml}
  legacy_code=$(yq -r '.legacy_code' "${file}")

  if ! check_yq "${file}" 'type == "!!map"' "root document must be a mapping"; then
    failed=1
  fi
  if ! check_yq "${file}" '((.legacy_code | type) == "!!str") and (.legacy_code | test("^SH-[0-9]{3}$"))' "legacy_code must look like SH-001"; then
    failed=1
  fi
  if ! check_yq "${file}" '((.legacy_name | type) == "!!str") and ((.legacy_name | length) > 0)' "legacy_name must be a non-empty string"; then
    failed=1
  fi
  if ! check_yq "${file}" '.new_category == "Correctness" or .new_category == "Style" or .new_category == "Performance" or .new_category == "Portability" or .new_category == "Security"' "new_category must be one of Correctness, Style, Performance, Portability, or Security"; then
    failed=1
  fi
  if ! check_yq "${file}" '((.new_code | type) == "!!str") and (.new_code | test("^[CSPXK][0-9]{3}$"))' "new_code must look like S001"; then
    failed=1
  fi
  if ! check_yq "${file}" '((.new_category == "Correctness") and (.new_code | test("^C[0-9]{3}$"))) or ((.new_category == "Style") and (.new_code | test("^S[0-9]{3}$"))) or ((.new_category == "Performance") and (.new_code | test("^P[0-9]{3}$"))) or ((.new_category == "Portability") and (.new_code | test("^X[0-9]{3}$"))) or ((.new_category == "Security") and (.new_code | test("^K[0-9]{3}$")))' "new_code prefix must match new_category"; then
    failed=1
  fi
  if ! check_yq "${file}" '.runtime_kind == "token" or .runtime_kind == "logical_line" or .runtime_kind == "ast" or .runtime_kind == "physical_line"' "runtime_kind must be one of token, logical_line, ast, or physical_line"; then
    failed=1
  fi
  if ! check_yq "${file}" '((.shellcheck_code | type) == "!!str") and (.shellcheck_code | test("^SC[0-9]+$"))' "shellcheck_code must look like SC2086"; then
    failed=1
  fi
  if ! check_yq "${file}" '((.shells | type) == "!!seq") and ((.shells | length) > 0) and ([.shells[] | (((type == "!!str") and (length > 0)))] | all)' "shells must be a non-empty list of strings"; then
    failed=1
  fi
  if ! check_yq "${file}" '((.description | type) == "!!str") and ((.description | length) > 0)' "description must be a non-empty string"; then
    failed=1
  fi
  if ! check_yq "${file}" '((.rationale | type) == "!!str") and ((.rationale | length) > 0)' "rationale must be a non-empty string"; then
    failed=1
  fi
  if ! check_yq "${file}" '((.source | type) == "!!map") and ((.source.shell_checks_rule | type) == "!!str") and ((.source.shell_checks_rule | length) > 0) and ((.source.shell_checks_example | type) == "!!str") and ((.source.shell_checks_example | length) > 0)' "source must include shell_checks_rule and shell_checks_example"; then
    failed=1
  fi
  if ! check_yq "${file}" '((.examples | type) == "!!seq") and ((.examples | length) > 0) and ([.examples[] | (((.kind | type) == "!!str") and ((.kind | length) > 0) and ((.source | type) == "!!str") and ((.source | length) > 0) and ((.code | type) == "!!str") and ((.code | length) > 0))] | all)' "examples must be a non-empty list of entries with kind, source, and code"; then
    failed=1
  fi

  if [[ "${stem}" != "${legacy_code}" ]]; then
    printf 'ERROR %s: filename must match legacy_code (%s)\n' "${basename}" "${legacy_code}" >&2
    failed=1
  fi

  if [[ ! -d "${shell_checks_root}" ]]; then
    printf 'ERROR %s: sibling shell-checks repo not found at %s\n' "${basename}" "${shell_checks_root}" >&2
    failed=1
  fi

  if [[ "${failed}" -eq 0 ]]; then
    rule_path=$(resolve_shell_checks_path "$(yq -r '.source.shell_checks_rule' "${file}")")
    example_path=$(resolve_shell_checks_path "$(yq -r '.source.shell_checks_example' "${file}")")

    if [[ ! -f "${rule_path}" ]]; then
      printf 'ERROR %s: referenced shell-checks rule file does not exist: %s\n' "${basename}" "${rule_path}" >&2
      failed=1
    fi

    if [[ ! -f "${example_path}" ]]; then
      printf 'ERROR %s: referenced shell-checks example file does not exist: %s\n' "${basename}" "${example_path}" >&2
      failed=1
    fi

    if ! doc_shells=$(normalized_list "${file}" '.shells'); then
      printf 'ERROR %s: could not read shells from docs rule\n' "${basename}" >&2
      failed=1
    fi

    if ! source_shells=$(normalized_list "${rule_path}" '.shells'); then
      printf 'ERROR %s: could not read shells from imported shell-checks rule %s\n' "${basename}" "${rule_path}" >&2
      failed=1
    fi

    if [[ "${failed}" -eq 0 && "${doc_shells}" != "${source_shells}" ]]; then
      printf 'ERROR %s: shells do not match imported shell-checks rule %s\n' "${basename}" "${rule_path}" >&2
      printf '  docs: %s\n' "$(printf '%s' "${doc_shells}" | paste -sd, -)" >&2
      printf '  src:  %s\n' "$(printf '%s' "${source_shells}" | paste -sd, -)" >&2
      failed=1
    fi

    while IFS= read -r example_source; do
      [[ -n "${example_source}" ]] || continue
      example_path=$(resolve_shell_checks_path "${example_source}")
      if [[ ! -f "${example_path}" ]]; then
        printf 'ERROR %s: example source does not exist in shell-checks: %s\n' "${basename}" "${example_path}" >&2
        failed=1
      fi
    done < <(yq -r '.examples[].source' "${file}")
  fi

  if [[ "${failed}" -eq 0 ]]; then
    printf 'OK %s\n' "${basename}"
  fi

  return "${failed}"
}

main() {
  local files=()
  local all_rule_files=("${script_dir}"/*.yaml)
  local file failed=0

  if [[ $# -gt 0 ]]; then
    files=("$@")
  else
    files=("${script_dir}"/*.yaml)
  fi

  if [[ ${#files[@]} -eq 0 ]]; then
    printf 'No rule YAML files found in %s\n' "${script_dir}" >&2
    exit 1
  fi

  for file in "${files[@]}"; do
    if ! validate_file "${file}"; then
      failed=1
    fi
  done

  if ! check_unique_field '.legacy_code' 'legacy_code' "${all_rule_files[@]}"; then
    failed=1
  fi

  if ! check_unique_field '.new_code' 'new_code' "${all_rule_files[@]}"; then
    failed=1
  fi

  if ! check_unique_field '.shellcheck_code' 'shellcheck_code' "${all_rule_files[@]}"; then
    failed=1
  fi

  exit "${failed}"
}

main "$@"
