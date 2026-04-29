use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use shuck_benchmark::configure_benchmark_allocator;
use shuck_indexer::Indexer;
use shuck_linter::{
    LinterSettings, Rule, RuleSet, ShellCheckCodeMap, ShellDialect, benchmark_collect_word_facts,
    lint_file_at_path_with_resolver_and_parse_result_and_directives, parse_directives,
};
use shuck_parser::parser::{ParseResult, Parser};
use shuck_semantic::{SemanticBuildOptions, SemanticModel, SourcePathResolver};

configure_benchmark_allocator!();

const LARGE_CORPUS_ROOT_ENV: &str = "SHUCK_LARGE_CORPUS_ROOT";
const LARGE_CORPUS_CACHE_DIR_NAME: &str = ".cache/large-corpus";
const AIRGEDDON_FIXTURE_NAME: &str = "v1s1t0r1sh3r3__airgeddon__airgeddon.sh";
const LANGUAGE_STRINGS_FIXTURE_NAME: &str = "v1s1t0r1sh3r3__airgeddon__language_strings.sh";

static LARGE_CORPUS_FIXTURES: OnceLock<Result<Arc<LoadedLargeCorpusFixtures>, String>> =
    OnceLock::new();

#[derive(Debug, Clone)]
struct LargeCorpusFixture {
    label: &'static str,
    path: PathBuf,
    cache_rel_path: PathBuf,
    shell: String,
    source: String,
}

impl LargeCorpusFixture {
    fn bytes(&self) -> u64 {
        self.source.len() as u64
    }
}

#[derive(Debug)]
struct LoadedLargeCorpusFixtures {
    airgeddon: Arc<LargeCorpusFixture>,
    language_strings: Arc<LargeCorpusFixture>,
    resolver: Arc<LargeCorpusPathResolver>,
}

#[derive(Debug)]
struct LargeCorpusPathResolver {
    cache_rel_by_path: HashMap<PathBuf, PathBuf>,
    path_by_cache_rel: HashMap<PathBuf, PathBuf>,
}

impl LargeCorpusPathResolver {
    fn new(fixtures: &[Arc<LargeCorpusFixture>]) -> Self {
        let mut cache_rel_by_path = HashMap::new();
        let mut path_by_cache_rel = HashMap::new();

        for fixture in fixtures {
            let canonical_path =
                fs::canonicalize(&fixture.path).unwrap_or_else(|_| fixture.path.clone());
            cache_rel_by_path.insert(fixture.path.clone(), fixture.cache_rel_path.clone());
            cache_rel_by_path.insert(canonical_path.clone(), fixture.cache_rel_path.clone());
            path_by_cache_rel.insert(fixture.cache_rel_path.clone(), canonical_path);
        }

        Self {
            cache_rel_by_path,
            path_by_cache_rel,
        }
    }
}

impl SourcePathResolver for LargeCorpusPathResolver {
    fn resolve_candidate_paths(&self, source_path: &Path, candidate: &str) -> Vec<PathBuf> {
        let Some(source_cache_rel_path) = self.cache_rel_by_path.get(source_path) else {
            return Vec::new();
        };

        let mut resolved = Vec::new();
        let mut seen = HashSet::new();
        for candidate_cache_rel_path in [
            resolve_large_corpus_candidate_cache_rel_path(source_cache_rel_path, candidate),
            resolve_large_corpus_repo_relative_candidate_cache_rel_path(
                source_cache_rel_path,
                candidate,
            ),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(path) = self.path_by_cache_rel.get(&candidate_cache_rel_path)
                && seen.insert(path.clone())
            {
                resolved.push(path.clone());
            }
        }

        resolved
    }
}

struct PreparedWordFactsInput {
    source: String,
    parse_result: ParseResult,
    indexer: Indexer,
    semantic: SemanticModel,
}

fn large_corpus_fixtures() -> Result<&'static Arc<LoadedLargeCorpusFixtures>, &'static str> {
    LARGE_CORPUS_FIXTURES
        .get_or_init(load_large_corpus_hotspot_fixtures)
        .as_ref()
        .map_err(|message| message.as_str())
}

fn load_large_corpus_hotspot_fixtures() -> Result<Arc<LoadedLargeCorpusFixtures>, String> {
    let corpus_dir = resolve_large_corpus_root().ok_or_else(|| {
        format!(
            "large corpus not found; set {LARGE_CORPUS_ROOT_ENV}, populate ./{LARGE_CORPUS_CACHE_DIR_NAME}, or place ../shell-checks next to the repo"
        )
    })?;
    let scripts_dir = corpus_dir.join("scripts");

    let airgeddon = Arc::new(load_large_corpus_fixture(
        &scripts_dir,
        AIRGEDDON_FIXTURE_NAME,
        "airgeddon",
    )?);
    let language_strings = Arc::new(load_large_corpus_fixture(
        &scripts_dir,
        LANGUAGE_STRINGS_FIXTURE_NAME,
        "language_strings",
    )?);
    let resolver = Arc::new(LargeCorpusPathResolver::new(&[
        Arc::clone(&airgeddon),
        Arc::clone(&language_strings),
    ]));

    Ok(Arc::new(LoadedLargeCorpusFixtures {
        airgeddon,
        language_strings,
        resolver,
    }))
}

fn resolve_large_corpus_root() -> Option<PathBuf> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let Some(repo_root) = manifest_dir.ancestors().nth(2) else {
        unreachable!("crates/shuck-benchmark should live two levels below repo root");
    };

    let root_hint = env::var(LARGE_CORPUS_ROOT_ENV)
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let candidates = root_hint.map(|hint| vec![hint]).unwrap_or_else(|| {
        vec![
            repo_root.join(LARGE_CORPUS_CACHE_DIR_NAME),
            repo_root.join("..").join("shell-checks"),
        ]
    });

    candidates
        .into_iter()
        .find_map(|candidate| normalize_large_corpus_root(&candidate))
}

fn normalize_large_corpus_root(root: &Path) -> Option<PathBuf> {
    if !root.is_dir() {
        return None;
    }

    let repo_corpus = root.join("corpus");
    if corpus_dir_looks_valid(&repo_corpus) {
        return Some(repo_corpus);
    }
    if corpus_dir_looks_valid(root) {
        return Some(root.to_path_buf());
    }

    None
}

fn corpus_dir_looks_valid(root: &Path) -> bool {
    root.join("scripts").is_dir()
}

fn load_large_corpus_fixture(
    scripts_dir: &Path,
    file_name: &str,
    label: &'static str,
) -> Result<LargeCorpusFixture, String> {
    let path = scripts_dir.join(file_name);
    let source = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let shell = resolve_large_corpus_shell(&path, source.as_bytes());

    Ok(LargeCorpusFixture {
        label,
        cache_rel_path: PathBuf::from(file_name),
        path,
        shell,
        source,
    })
}

fn resolve_large_corpus_shell(path: &Path, source: &[u8]) -> String {
    let source = String::from_utf8_lossy(source);
    let source = source.strip_prefix('\u{feff}').unwrap_or(source.as_ref());
    let trimmed_first_line = source
        .lines()
        .next()
        .map(|line| line.trim_start().to_ascii_lowercase())
        .unwrap_or_default();

    if trimmed_first_line.starts_with("#compdef") || trimmed_first_line.starts_with("#autoload") {
        return "zsh".into();
    }

    match ShellDialect::infer(source, Some(path)) {
        ShellDialect::Bash => "bash".into(),
        ShellDialect::Ksh | ShellDialect::Mksh => "ksh".into(),
        ShellDialect::Zsh => "zsh".into(),
        ShellDialect::Sh | ShellDialect::Dash => "sh".into(),
        ShellDialect::Unknown => "sh".into(),
    }
}

fn parse_large_corpus_fixture(fixture: &LargeCorpusFixture) -> ParseResult {
    let shell = ShellDialect::from_name(&fixture.shell);
    Parser::with_dialect(&fixture.source, shell.parser_dialect()).parse()
}

fn prepare_word_facts_input(
    fixture: &LargeCorpusFixture,
    resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> PreparedWordFactsInput {
    let parse_result = parse_large_corpus_fixture(fixture);
    let indexer = Indexer::new(&fixture.source, &parse_result);
    let semantic = SemanticModel::build_with_options(
        &parse_result.file,
        &fixture.source,
        &indexer,
        SemanticBuildOptions {
            source_path: Some(&fixture.path),
            source_path_resolver: resolver,
            file_entry_contract: None,
            file_entry_contract_collector: None,
            analyzed_paths: None,
            shell_profile: Some(ShellDialect::from_name(&fixture.shell).shell_profile()),
            resolve_source_closure: true,
        },
    );

    PreparedWordFactsInput {
        source: fixture.source.clone(),
        parse_result,
        indexer,
        semantic,
    }
}

fn lint_large_corpus_fixture(
    fixture: &LargeCorpusFixture,
    resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> usize {
    lint_large_corpus_fixture_with_settings(
        fixture,
        resolver,
        large_corpus_default_settings(fixture),
    )
}

fn lint_large_corpus_fixture_with_settings(
    fixture: &LargeCorpusFixture,
    resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
    settings: LinterSettings,
) -> usize {
    let settings = settings
        .with_shell(ShellDialect::from_name(&fixture.shell))
        .with_analyzed_paths([fixture.path.clone()]);
    let parse_result = parse_large_corpus_fixture(fixture);
    let indexer = Indexer::new(&fixture.source, &parse_result);
    let shellcheck_map = ShellCheckCodeMap::default();
    let directives = parse_directives(
        &fixture.source,
        &parse_result.file,
        indexer.comment_index(),
        &shellcheck_map,
    );
    let diagnostics = lint_file_at_path_with_resolver_and_parse_result_and_directives(
        &parse_result,
        &fixture.source,
        &indexer,
        &settings,
        &directives,
        Some(&fixture.path),
        resolver,
    );

    black_box(diagnostics.len())
}

fn large_corpus_default_settings(fixture: &LargeCorpusFixture) -> LinterSettings {
    LinterSettings::default()
        .with_shell(ShellDialect::from_name(&fixture.shell))
        .with_analyzed_paths([fixture.path.clone()])
}

fn large_corpus_c100_only_settings() -> LinterSettings {
    LinterSettings::for_rule(Rule::QuotedBashSource)
}

fn large_corpus_without_c100_settings() -> LinterSettings {
    let mut settings = LinterSettings::default();
    settings.rules = settings
        .rules
        .subtract(&RuleSet::from_iter([Rule::QuotedBashSource]));
    settings
}

fn large_corpus_without_source_closure_settings(fixture: &LargeCorpusFixture) -> LinterSettings {
    large_corpus_default_settings(fixture).with_resolve_source_closure(false)
}

fn build_large_corpus_word_facts(input: &PreparedWordFactsInput) -> usize {
    black_box(
        benchmark_collect_word_facts(&input.parse_result.file, &input.source, &input.semantic)
            + input.indexer.comment_index().comments().len(),
    )
}

fn resolve_large_corpus_candidate_cache_rel_path(
    source_cache_rel_path: &Path,
    candidate: &str,
) -> Option<PathBuf> {
    let candidate_path = Path::new(candidate);
    if candidate_path.is_absolute() {
        return None;
    }

    let mut resolved = source_cache_rel_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut saw_component = false;

    for component in candidate_path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !resolved.pop() {
                    return None;
                }
                saw_component = true;
            }
            std::path::Component::Normal(part) => {
                resolved.push(part);
                saw_component = true;
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return None,
        }
    }

    saw_component.then_some(resolved)
}

fn resolve_large_corpus_repo_relative_candidate_cache_rel_path(
    source_cache_rel_path: &Path,
    candidate: &str,
) -> Option<PathBuf> {
    if source_cache_rel_path.components().count() != 1 {
        return None;
    }

    let source_name = source_cache_rel_path.file_name()?.to_str()?;
    let mut source_parts = source_name.split("__");
    let owner = source_parts.next()?;
    let repo = source_parts.next()?;

    let candidate_path = Path::new(candidate);
    if candidate_path.is_absolute() {
        return None;
    }

    let mut flattened = Vec::new();
    for component in candidate_path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => return None,
            std::path::Component::Normal(part) => {
                flattened.push(part.to_string_lossy().into_owned());
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return None,
        }
    }

    (!flattened.is_empty())
        .then(|| PathBuf::from(format!("{owner}__{repo}__{}", flattened.join("__"))))
}

fn bench_large_corpus_linter(c: &mut Criterion) {
    let Ok(fixtures) = large_corpus_fixtures() else {
        let Err(message) = large_corpus_fixtures() else {
            unreachable!();
        };
        eprintln!("skipping large corpus hotspot benches: {message}");
        return;
    };

    let mut group = c.benchmark_group("large_corpus_linter");
    group.sample_size(10);

    for fixture in [&fixtures.airgeddon, &fixtures.language_strings] {
        group.throughput(Throughput::Bytes(fixture.bytes()));
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture.label),
            fixture,
            |b, fixture| {
                let resolver = fixtures.resolver.clone();
                b.iter(|| {
                    lint_large_corpus_fixture(
                        fixture,
                        Some(resolver.as_ref() as &(dyn SourcePathResolver + Send + Sync)),
                    )
                });
            },
        );
    }

    group.finish();
}

fn bench_large_corpus_linter_rule_splits(c: &mut Criterion) {
    let Ok(fixtures) = large_corpus_fixtures() else {
        let Err(message) = large_corpus_fixtures() else {
            unreachable!();
        };
        eprintln!("skipping large corpus hotspot benches: {message}");
        return;
    };

    let fixture = &fixtures.airgeddon;
    let mut group = c.benchmark_group("large_corpus_linter_rule_splits");
    group.sample_size(10);
    group.throughput(Throughput::Bytes(fixture.bytes()));

    group.bench_with_input(
        BenchmarkId::from_parameter("airgeddon/c100_only"),
        fixture,
        |b, fixture| {
            let resolver = fixtures.resolver.clone();
            let settings = large_corpus_c100_only_settings();
            b.iter(|| {
                lint_large_corpus_fixture_with_settings(
                    fixture,
                    Some(resolver.as_ref() as &(dyn SourcePathResolver + Send + Sync)),
                    settings.clone(),
                )
            });
        },
    );

    group.bench_with_input(
        BenchmarkId::from_parameter("airgeddon/without_c100"),
        fixture,
        |b, fixture| {
            let resolver = fixtures.resolver.clone();
            let settings = large_corpus_without_c100_settings();
            b.iter(|| {
                lint_large_corpus_fixture_with_settings(
                    fixture,
                    Some(resolver.as_ref() as &(dyn SourcePathResolver + Send + Sync)),
                    settings.clone(),
                )
            });
        },
    );

    group.finish();
}

fn bench_large_corpus_linter_source_closure_splits(c: &mut Criterion) {
    let Ok(fixtures) = large_corpus_fixtures() else {
        let Err(message) = large_corpus_fixtures() else {
            unreachable!();
        };
        eprintln!("skipping large corpus hotspot benches: {message}");
        return;
    };

    let fixture = &fixtures.airgeddon;
    let mut group = c.benchmark_group("large_corpus_linter_source_closure_splits");
    group.sample_size(10);
    group.throughput(Throughput::Bytes(fixture.bytes()));

    group.bench_with_input(
        BenchmarkId::from_parameter("airgeddon/with_source_closure"),
        fixture,
        |b, fixture| {
            let resolver = fixtures.resolver.clone();
            let settings = large_corpus_default_settings(fixture);
            b.iter(|| {
                lint_large_corpus_fixture_with_settings(
                    fixture,
                    Some(resolver.as_ref() as &(dyn SourcePathResolver + Send + Sync)),
                    settings.clone(),
                )
            });
        },
    );

    group.bench_with_input(
        BenchmarkId::from_parameter("airgeddon/without_source_closure"),
        fixture,
        |b, fixture| {
            let resolver = fixtures.resolver.clone();
            let settings = large_corpus_without_source_closure_settings(fixture);
            b.iter(|| {
                lint_large_corpus_fixture_with_settings(
                    fixture,
                    Some(resolver.as_ref() as &(dyn SourcePathResolver + Send + Sync)),
                    settings.clone(),
                )
            });
        },
    );

    group.finish();
}

fn bench_large_corpus_word_facts(c: &mut Criterion) {
    let Ok(fixtures) = large_corpus_fixtures() else {
        let Err(message) = large_corpus_fixtures() else {
            unreachable!();
        };
        eprintln!("skipping large corpus hotspot benches: {message}");
        return;
    };

    let mut group = c.benchmark_group("large_corpus_word_facts");
    group.sample_size(10);
    group.throughput(Throughput::Bytes(fixtures.language_strings.bytes()));
    group.bench_with_input(
        BenchmarkId::from_parameter(fixtures.language_strings.label),
        &fixtures.language_strings,
        |b, fixture| {
            b.iter_batched(
                || prepare_word_facts_input(fixture, None),
                |input| {
                    black_box(build_large_corpus_word_facts(&input));
                },
                BatchSize::LargeInput,
            );
        },
    );
    group.finish();
}

criterion_group!(
    benches,
    bench_large_corpus_linter,
    bench_large_corpus_linter_rule_splits,
    bench_large_corpus_linter_source_closure_splits,
    bench_large_corpus_word_facts
);
criterion_main!(benches);
