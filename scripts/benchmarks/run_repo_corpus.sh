#!/bin/sh
# Clone curated upstream repos and benchmark shuck vs shellcheck on the
# real on-disk layout (so source-following, project root resolution, and
# .shellcheckrc handling all behave the way they would for an end user).
#
# For each repo:
#   1. Clone (shallow) into $clone_dir/<repo_key>/, pinned to the SHA from
#      .cache/large-corpus/manifest.yaml when fetchable, else default HEAD.
#   2. Enumerate shell scripts via `git ls-files`: extension match for
#      *.sh, *.bash, *.ksh, plus extensionless executables whose first
#      line is a recognized shell shebang. Excludes *.zsh and any path
#      with whitespace.
#   3. Run hyperfine with shuck and shellcheck pointed at the same filelist.
#
# Outputs go under .cache/repo-corpus/<owner>__<name>/{filelist.txt,bench.json,meta.json}.
#
# Environment overrides:
#   SHUCK_BENCH_REPO_TRUNCATE   max files per repo (default 4000)
#   SHUCK_BENCH_REPO_WARMUP     hyperfine warmup runs (default 1)
#   SHUCK_BENCH_REPO_RUNS       hyperfine measured runs (default 3)
#   SHUCK_BENCH_REPO_REPOS      override the repo list (newline-separated)
#   SHUCK_BENCHMARK_REPO_CLONE_DIR  where to cache clones (default $TMPDIR/shuck-bench-repos)
#   SHUCK_BENCHMARK_MANIFEST    manifest path for SHA pinning (optional)
set -eu

repo_root=${SHUCK_BENCHMARK_REPO_ROOT:-$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)}
manifest_file=${SHUCK_BENCHMARK_MANIFEST:-"$repo_root/.cache/large-corpus/manifest.yaml"}
# Cache clones in $HOME so reruns reuse them. We deliberately don't follow
# $TMPDIR — nix develop overrides it to a per-session path, which would
# force a re-clone on every invocation. Avoid any path containing `.cache`
# as a component: shuck's discovery layer hardcodes that as ignored, so
# cloning under e.g. `~/.cache/shuck-bench-repos` would silently filter
# every file out of the bench.
clone_dir=${SHUCK_BENCHMARK_REPO_CLONE_DIR:-"$HOME/.shuck-bench-repos"}
target_dir=${CARGO_TARGET_DIR:-"$repo_root/target"}
shuck_bin=${SHUCK_BENCHMARK_SHUCK_BIN:-"$target_dir/release/shuck"}
out_dir=${SHUCK_BENCHMARK_REPO_OUTPUT_DIR:-"$repo_root/.cache/repo-corpus"}
truncate_limit=${SHUCK_BENCH_REPO_TRUNCATE:-4000}
warmup=${SHUCK_BENCH_REPO_WARMUP:-1}
runs=${SHUCK_BENCH_REPO_RUNS:-3}

if [ ! -x "$shuck_bin" ]; then
    echo "Error: shuck binary not found at $shuck_bin" >&2
    echo "Run 'cargo build --release -p shuck-cli' first." >&2
    exit 1
fi

for tool in hyperfine shellcheck git; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "Error: $tool not found on PATH" >&2
        exit 1
    fi
done

default_repos="
SlackBuildsOrg/slackbuilds
bitnami/containers
acmesh-official/acme.sh
v1s1t0r1sh3r3/airgeddon
Bash-it/bash-it
nvm-sh/nvm
super-linter/super-linter
bats-core/bats-core
dylanaraps/neofetch
"
# Repos intentionally not benched here:
# - alpinelinux/aports: scripts are APKBUILD files (extensionless, sourced
#   without shebangs) so our discovery picks up only a handful of helper
#   .sh scripts and the bench measures almost nothing.
# - CISOfy/lynis: same shape. Logic lives in non-executable extensionless
#   files under include/ which fail the shebang gate.
repos=${SHUCK_BENCH_REPO_REPOS:-$default_repos}

# Look up a repo's pinned commit SHA in the manifest. The manifest is the
# minimal `- repo: foo/bar\n  commit: <sha>\n  ...` two-level YAML the corpus
# downloader emits, so we hand-parse rather than pulling in a YAML lib.
manifest_commit() {
    target=$1
    [ -f "$manifest_file" ] || { printf ''; return; }
    awk -v target="$target" '
        /^  - repo:/ {
            # field $3 is the repo slug; trim trailing whitespace
            current=$3
            sub(/[[:space:]]+$/, "", current)
            next
        }
        /^    commit:/ && current==target {
            sub(/^[[:space:]]+commit:[[:space:]]*/, "")
            print
            exit
        }
    ' "$manifest_file"
}

# Ensure $clone_path is checked out at $desired_commit (or HEAD if unpinned).
# Idempotent: re-running with the same target SHA is a no-op.
ensure_clone() {
    repo=$1
    clone_path=$2
    desired_commit=$3

    if [ ! -d "$clone_path/.git" ]; then
        echo "    cloning https://github.com/$repo.git"
        rm -rf "$clone_path"
        git clone --depth=1 --no-tags -q "https://github.com/$repo.git" "$clone_path"
    fi

    if [ -n "$desired_commit" ]; then
        current=$(git -C "$clone_path" rev-parse HEAD 2>/dev/null || echo "")
        if [ "$current" = "$desired_commit" ]; then
            return
        fi
        if ! git -C "$clone_path" cat-file -e "$desired_commit" 2>/dev/null; then
            echo "    fetching $desired_commit"
            if ! git -C "$clone_path" fetch --depth=1 -q origin "$desired_commit" 2>/dev/null; then
                echo "    warning: cannot fetch $desired_commit; using current HEAD"
                return
            fi
        fi
        # -f because some repos (e.g., SlackBuildsOrg/slackbuilds) ship
        # case-colliding paths that don't survive a checkout on
        # case-insensitive filesystems like APFS; git then sees the working
        # tree as "dirty" and refuses to switch without -f. We lose at most
        # one file from each collision pair, which is irrelevant for a
        # benchmark of thousands of scripts.
        git -C "$clone_path" -c advice.detachedHead=false checkout -f -q "$desired_commit"
    fi
}

# Print the absolute paths of shell scripts in $clone_path, one per line:
# extension match (*.sh, *.bash, *.ksh) plus extensionless executables with
# a recognized shell shebang. Excludes *.zsh, paths with whitespace, and
# anything outside the working tree (git ls-files handles .git/ and submodules).
build_filelist() {
    clone_path=$1

    git -C "$clone_path" ls-files \
        | awk -F/ '
            /[[:space:]]/ { next }
            /\.zsh$/      { next }
            /\.(sh|bash|ksh)$/ { print "DIRECT\t" $0; next }
            { if ($NF !~ /\./) print "MAYBE\t" $0 }
        ' \
        | while IFS=$(printf '\t') read -r kind rel; do
            [ -n "$rel" ] || continue
            full="$clone_path/$rel"
            # Skip symlinks: shuck canonicalizes through them and then
            # complains the resolved target is outside the inferred project
            # root (e.g., bats-core ships folder1/setup_suite.bash linking
            # to ../setup_suite.bash). The link's target shows up as its
            # own entry in `git ls-files` if it's tracked, so we don't
            # lose coverage by skipping the link itself.
            [ -f "$full" ] && [ ! -L "$full" ] || continue
            case "$kind" in
                DIRECT)
                    printf '%s\n' "$full"
                    ;;
                MAYBE)
                    [ -x "$full" ] || continue
                    # Only the first line matters; head -n 1 reads at most a
                    # buffer, so cost stays bounded on huge files.
                    if head -n 1 "$full" 2>/dev/null \
                        | grep -Eq '^#![[:space:]]*(/[^[:space:]]*/)?(env[[:space:]]+)?(ba|k)?sh([[:space:]]|$)'; then
                        printf '%s\n' "$full"
                    fi
                    ;;
            esac
        done \
        | sort -u
}

mkdir -p "$out_dir" "$clone_dir"

for repo in $repos; do
    owner=$(echo "$repo" | cut -d/ -f1)
    name=$(echo "$repo" | cut -d/ -f2)
    repo_key="${owner}__${name}"
    repo_out="$out_dir/$repo_key"
    clone_path="$clone_dir/$repo_key"
    filelist="$repo_out/filelist.txt"

    mkdir -p "$repo_out"

    desired_commit=$(manifest_commit "$repo" || true)
    if [ -n "$desired_commit" ]; then
        echo "==> Preparing $repo @ $(printf '%.7s' "$desired_commit")"
    else
        echo "==> Preparing $repo (HEAD)"
    fi
    ensure_clone "$repo" "$clone_path" "$desired_commit"

    actual_commit=$(git -C "$clone_path" rev-parse HEAD)
    actual_commit_short=$(git -C "$clone_path" rev-parse --short=7 HEAD)

    build_filelist "$clone_path" > "$filelist.full"

    # SlackBuilds is dominated by tiny doinst.sh post-install snippets;
    # alphabetical ordering means truncation keeps an arbitrary prefix
    # rather than the heaviest scripts. Sort by file size descending so
    # the truncate cap (and the workload character) skews toward the
    # largest, most representative scripts.
    case "$repo_key" in
        SlackBuildsOrg__slackbuilds)
            sorted_tmp=$(mktemp)
            # `xargs wc -c` emits "<bytes> <path>" per file plus a trailing
            # "<total> total" summary (one per xargs batch). Filter the
            # summary lines by exact path match.
            xargs wc -c < "$filelist.full" \
                | awk 'NF >= 2 && $NF != "total" {
                    size = $1; $1 = ""; sub(/^[[:space:]]+/, "")
                    print size "\t" $0
                  }' \
                | sort -rn -k1,1 \
                | cut -f2- \
                > "$sorted_tmp"
            mv "$sorted_tmp" "$filelist.full"
            ;;
    esac

    available=$(wc -l < "$filelist.full" | tr -d ' ')
    head -n "$truncate_limit" "$filelist.full" > "$filelist"
    rm -f "$filelist.full"

    count=$(wc -l < "$filelist" | tr -d ' ')
    if [ "$count" = "0" ]; then
        echo "    no shell files found in clone, skipping"
        continue
    fi

    # Sum per-file `wc` counts directly. xargs may invoke wc multiple times
    # when the filelist exceeds ARG_MAX, and each invocation emits its own
    # "total" summary line, so a `tail -1` over the combined stream only
    # captures the last batch's subtotal.
    total_bytes=$(xargs wc -c < "$filelist" | awk '$NF != "total" && NF >= 2 { sum += $1 } END { print sum + 0 }')
    total_lines=$(xargs wc -l < "$filelist" | awk '$NF != "total" && NF >= 2 { sum += $1 } END { print sum + 0 }')
    truncated=false
    if [ "$count" -lt "$available" ]; then
        truncated=true
    fi

    echo "==> Benchmarking $repo @ $actual_commit_short: $count files ($total_bytes bytes, $total_lines lines)"
    if [ "$truncated" = "true" ]; then
        echo "    (truncated from $available to $truncate_limit per SHUCK_BENCH_REPO_TRUNCATE)"
    fi

    # xargs reads the filelist from stdin and exec's the tool with all paths
    # as a single argv (or batches if ARG_MAX would be exceeded). Output is
    # dropped so hyperfine is timing lint work, not stdout I/O.
    hyperfine \
        --ignore-failure \
        --warmup "$warmup" \
        --runs "$runs" \
        --export-json "$repo_out/bench.json" \
        -n "shuck" "xargs '$shuck_bin' check --no-cache --select ALL <'$filelist' >/dev/null 2>&1" \
        -n "shellcheck" "xargs shellcheck --enable=all --severity=style <'$filelist' >/dev/null 2>&1"

    cat > "$repo_out/meta.json" <<EOF
{
  "repo": "$repo",
  "repoKey": "$repo_key",
  "commit": "$actual_commit",
  "commitShort": "$actual_commit_short",
  "fileCount": $count,
  "availableFileCount": $available,
  "totalBytes": $total_bytes,
  "totalLines": $total_lines,
  "truncated": $truncated,
  "truncateLimit": $truncate_limit
}
EOF
done

echo ""
echo "==> Done. Per-repo results in $out_dir"
echo "==> Cached clones in $clone_dir (delete to force re-clone)"
