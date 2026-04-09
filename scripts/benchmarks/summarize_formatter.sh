#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
cache_dir=${1:-"$repo_root/.cache"}

if ! command -v yq >/dev/null 2>&1; then
    echo "ERROR: yq is required to summarize formatter benchmarks." >&2
    exit 1
fi

found=0
for path in "$cache_dir"/bench-format-*.json; do
    if [ ! -s "$path" ]; then
        continue
    fi
    found=1

    name=$(basename "$path" .json | sed 's/^bench-format-//')
    shuck_mean=
    shuck_exit_codes=
    shfmt_mean=
    shfmt_exit_codes=

    while IFS='	' read -r command mean exit_codes; do
        case "$command" in
            shuck-format/*|shuck-format)
                shuck_mean=$mean
                shuck_exit_codes=$exit_codes
                ;;
            shfmt/*|shfmt)
                shfmt_mean=$mean
                shfmt_exit_codes=$exit_codes
                ;;
        esac
    done <<EOF
$(yq -r '.results[] | [.command, .mean, (.exit_codes | map(tostring) | join(","))] | @tsv' "$path")
EOF

    if [ -z "$shuck_mean" ] || [ -z "$shfmt_mean" ]; then
        echo "$name: incomplete benchmark data"
        continue
    fi

    ratio=$(awk "BEGIN { printf \"%.2fx\", $shuck_mean / $shfmt_mean }")
    shuck_ms=$(awk "BEGIN { printf \"%.2f\", $shuck_mean * 1000 }")
    shfmt_ms=$(awk "BEGIN { printf \"%.2f\", $shfmt_mean * 1000 }")

    shuck_note=
    shfmt_note=
    case ",$shuck_exit_codes," in
        *,124,*) shuck_note=" timeout" ;;
    esac
    case ",$shfmt_exit_codes," in
        *,124,*) shfmt_note=" timeout" ;;
    esac

    printf '%-20s  shuck=%8sms%-8s shfmt=%8sms%-8s ratio=%s\n' \
        "$name" "$shuck_ms" "$shuck_note" "$shfmt_ms" "$shfmt_note" "$ratio"
done

if [ "$found" -eq 0 ]; then
    echo "No formatter benchmark exports found under $cache_dir"
fi
