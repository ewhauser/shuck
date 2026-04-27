use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use shuck_benchmark::configure_benchmark_allocator;
use shuck_linter::BenchmarkCasePatternMatcher as Matcher;

configure_benchmark_allocator!();

struct PatternPair {
    name: &'static str,
    left: &'static str,
    right: &'static str,
}

const PAIRS: &[PatternPair] = &[
    // Pure literals — fixed-length fast path; no NFA work.
    PatternPair {
        name: "literal/equal",
        left: "abcdef",
        right: "abcdef",
    },
    PatternPair {
        name: "literal/diff",
        left: "abcdef",
        right: "abcxyz",
    },
    PatternPair {
        name: "literal/long_equal",
        left: "configure_benchmark_allocator",
        right: "configure_benchmark_allocator",
    },
    // Single '?' — fixed length, fast path.
    PatternPair {
        name: "qmark/single",
        left: "?",
        right: "a",
    },
    PatternPair {
        name: "qmark/triple_dot_sh",
        left: "???.sh",
        right: "foo.sh",
    },
    PatternPair {
        name: "qmark/all_qmark",
        left: "????????",
        right: "abcdefgh",
    },
    // Single-'*' simple-glob shapes — should hit PR #663's fast path.
    PatternPair {
        name: "star/bare",
        left: "*",
        right: "anything-here",
    },
    PatternPair {
        name: "star/star_vs_star",
        left: "*",
        right: "*",
    },
    PatternPair {
        name: "star/prefix",
        left: "foo*",
        right: "foobar",
    },
    PatternPair {
        name: "star/suffix",
        left: "*.txt",
        right: "readme.txt",
    },
    PatternPair {
        name: "star/middle",
        left: "foo*bar",
        right: "fooXYZbar",
    },
    PatternPair {
        name: "star/prefix_vs_prefix",
        left: "foo*",
        right: "foobar*",
    },
    PatternPair {
        name: "star/suffix_vs_suffix",
        left: "*.sh",
        right: "*.bash",
    },
    PatternPair {
        name: "star/disjoint_prefix",
        left: "foo*",
        right: "bar*",
    },
    // Multi-'*' patterns — fall through to NFA product walk.
    PatternPair {
        name: "multistar/contains",
        left: "*foo*",
        right: "abcfoodef",
    },
    PatternPair {
        name: "multistar/two_each",
        left: "*foo*",
        right: "*bar*",
    },
    PatternPair {
        name: "multistar/three_segments",
        left: "*a*b*c*",
        right: "axxbyycyy",
    },
    PatternPair {
        name: "multistar/repeated_token",
        left: "*aa*",
        right: "aaaaaaaaaa",
    },
    // Mixed '*' and '?' — NFA walk, more expensive.
    PatternPair {
        name: "mixed/qstar_around_lit",
        left: "?*foo*?",
        right: "abcfoodef",
    },
    PatternPair {
        name: "mixed/lit_q_star",
        left: "foo?*bar",
        right: "fooXbar",
    },
    PatternPair {
        name: "mixed/star_q_lit_q",
        left: "*?.t?t",
        right: "ab.txt",
    },
    PatternPair {
        name: "mixed/long_alternation",
        left: "a*?b*?c*?d",
        right: "axbycdzdz",
    },
    // Realistic shell case-arm shapes.
    PatternPair {
        name: "shell/help",
        left: "--help",
        right: "--help",
    },
    PatternPair {
        name: "shell/h_short",
        left: "-h",
        right: "--help",
    },
    PatternPair {
        name: "shell/dash_anything",
        left: "-*",
        right: "-h",
    },
    PatternPair {
        name: "shell/double_dash_anything",
        left: "--*",
        right: "--verbose",
    },
    PatternPair {
        name: "shell/sh_ext",
        left: "*.sh",
        right: "*.bash",
    },
    PatternPair {
        name: "shell/script_ext",
        left: "*.sh",
        right: "*.zsh",
    },
    PatternPair {
        name: "shell/path_under",
        left: "/usr/local/*",
        right: "/usr/local/bin/foo",
    },
    PatternPair {
        name: "shell/wildcard_anywhere",
        left: "*",
        right: "/var/log/messages",
    },
];

fn build_matchers() -> Vec<(&'static str, Matcher, Matcher)> {
    PAIRS
        .iter()
        .map(|pair| {
            let left = Matcher::from_glob(pair.left)
                .unwrap_or_else(|| panic!("not statically analyzable: {}", pair.left));
            let right = Matcher::from_glob(pair.right)
                .unwrap_or_else(|| panic!("not statically analyzable: {}", pair.right));
            (pair.name, left, right)
        })
        .collect()
}

fn bench_subsumes(c: &mut Criterion) {
    let mut group = c.benchmark_group("case_pattern_subsumes");
    for (name, left, right) in build_matchers() {
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            &(left, right),
            |b, (l, r)| {
                b.iter(|| {
                    let a = black_box(l).subsumes(black_box(r));
                    let b = black_box(r).subsumes(black_box(l));
                    black_box(a) || black_box(b)
                });
            },
        );
    }
    group.finish();
}

fn bench_intersects(c: &mut Criterion) {
    let mut group = c.benchmark_group("case_pattern_intersects");
    for (name, left, right) in build_matchers() {
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            &(left, right),
            |b, (l, r)| {
                b.iter(|| black_box(black_box(l).intersects(black_box(r))));
            },
        );
    }
    group.finish();
}

fn bench_from_glob(c: &mut Criterion) {
    let mut group = c.benchmark_group("case_pattern_from_glob");
    for pair in PAIRS {
        group.bench_with_input(
            BenchmarkId::from_parameter(pair.name),
            pair.left,
            |b, glob| {
                b.iter(|| black_box(Matcher::from_glob(black_box(glob))));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_subsumes, bench_intersects, bench_from_glob);
criterion_main!(benches);
