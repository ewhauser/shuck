use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Instant;

use shuck_indexer::Indexer;
use shuck_linter::{
    LinterSettings, RuleSelector, RuleSet, ShellCheckCodeMap, ShellDialect,
    lint_file_at_path_with_resolver_and_parse_result_with_comment_directives,
};
use shuck_parser::parser::Parser;
use shuck_semantic::SourcePathResolver;

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

const LARGE_CORPUS_ROOT_ENV: &str = "SHUCK_LARGE_CORPUS_ROOT";
const LARGE_CORPUS_CACHE_DIR_NAME: &str = ".cache/large-corpus";
const LARGE_CORPUS_STATIC_IGNORE_SUFFIXES: &[&str] = &[
    "super-linter__super-linter__test__linters__bash__shell_bad_1.sh",
    "super-linter__super-linter__test__linters__bash_exec__shell_bad_1.sh",
    "alpinelinux__aports__community__starship__starship.plugin.zsh",
    "CISOfy__lynis__include__tests_ports_packages",
    "google__oss-fuzz__infra__chronos__coverage_test_collection.py",
    "moovweb__gvm__examples__native__configure",
    "moovweb__gvm__examples__native__ltmain.sh",
    "ohmyzsh__ohmyzsh__plugins__alias-finder__tests__test_run.sh",
];
const LARGE_CORPUS_STATIC_ZSH_OVERRIDE_SUFFIXES: &[&str] = &[
    "ohmyzsh__ohmyzsh__oh-my-zsh.sh",
    "ohmyzsh__ohmyzsh__tools__check_for_upgrade.sh",
];

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug)]
struct Args {
    fixture: Option<String>,
    iterations: usize,
    corpus_root: Option<PathBuf>,
    fixture_manifest: Option<PathBuf>,
    write_fixture_manifest: Option<PathBuf>,
    rules: Option<RuleSet>,
    resolve_source_closure: bool,
}

#[derive(Debug, Clone)]
struct LargeCorpusFixture {
    path: PathBuf,
    canonical_path: PathBuf,
    cache_rel_path: PathBuf,
    shell: String,
}

impl LargeCorpusFixture {
    fn cache_rel_path_key(&self) -> String {
        normalize_cache_rel_path(&self.cache_rel_path)
    }
}

#[derive(Debug)]
struct LargeCorpusPathResolver {
    cache_rel_by_path: HashMap<PathBuf, PathBuf>,
    path_by_cache_rel: HashMap<PathBuf, PathBuf>,
}

impl LargeCorpusPathResolver {
    fn new(fixtures: &[LargeCorpusFixture]) -> Self {
        let mut cache_rel_by_path = HashMap::new();
        let mut path_by_cache_rel = HashMap::new();

        for fixture in fixtures {
            cache_rel_by_path.insert(fixture.path.clone(), fixture.cache_rel_path.clone());
            cache_rel_by_path.insert(
                fixture.canonical_path.clone(),
                fixture.cache_rel_path.clone(),
            );
            path_by_cache_rel.insert(
                fixture.cache_rel_path.clone(),
                fixture.canonical_path.clone(),
            );
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
                && !resolved.contains(path)
            {
                resolved.push(path.clone());
            }
        }

        resolved
    }
}

fn main() -> Result<()> {
    #[cfg(feature = "dhat-heap")]
    let _dhat = dhat::Profiler::new_heap();

    let Some(args) = parse_args()? else {
        return Ok(());
    };
    let corpus_dir = resolve_large_corpus_root(args.corpus_root.as_deref()).ok_or_else(|| {
        format!(
            "large corpus not found; set {LARGE_CORPUS_ROOT_ENV}, populate ./{LARGE_CORPUS_CACHE_DIR_NAME}, or pass --root"
        )
    })?;
    let fixtures = if let Some(manifest) = args.fixture_manifest.as_deref() {
        read_fixture_manifest(manifest)?
    } else {
        collect_fixtures(&corpus_dir)
    };
    if fixtures.is_empty() {
        return Err(format!(
            "no fixtures found in {}",
            corpus_dir.join("scripts").display()
        )
        .into());
    }
    if let Some(manifest) = args.write_fixture_manifest.as_deref() {
        write_fixture_manifest(manifest, &fixtures)?;
        eprintln!(
            "wrote {} fixture(s) to {}",
            fixtures.len(),
            manifest.display()
        );
        return Ok(());
    }

    let fixture_name = args.fixture.as_ref().ok_or("missing fixture name")?;
    let fixture = find_fixture(&fixtures, fixture_name)
        .ok_or_else(|| format!("fixture `{fixture_name}` not found in large corpus"))?
        .clone();
    let resolver = LargeCorpusPathResolver::new(&fixtures);
    let settings = build_linter_settings(args.rules, args.resolve_source_closure);

    eprintln!(
        "profiling {} ({}) for {} iteration(s)",
        fixture.cache_rel_path_key(),
        fixture.shell,
        args.iterations
    );
    eprintln!("corpus root: {}", corpus_dir.display());

    let start = Instant::now();
    let mut diagnostics_len = 0usize;
    for _ in 0..args.iterations {
        diagnostics_len = black_box(run_large_corpus_fixture(&fixture, &settings, &resolver)?);
    }
    let elapsed = start.elapsed();

    eprintln!(
        "done: diagnostics={} elapsed={:.3}s avg={:.3}ms",
        diagnostics_len,
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000.0 / args.iterations as f64
    );

    Ok(())
}

fn parse_args() -> Result<Option<Args>> {
    let mut fixture = None;
    let mut iterations = 1usize;
    let mut corpus_root = None;
    let mut fixture_manifest = None;
    let mut write_fixture_manifest = None;
    let mut rules = None;
    let mut resolve_source_closure = true;
    let mut values = env::args().skip(1);

    while let Some(arg) = values.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                return Ok(None);
            }
            "--iterations" => {
                iterations = parse_positive_usize("--iterations", values.next())?;
            }
            "--root" => {
                corpus_root = Some(PathBuf::from(
                    values.next().ok_or("missing value after --root")?,
                ));
            }
            "--fixture-manifest" => {
                fixture_manifest = Some(PathBuf::from(
                    values
                        .next()
                        .ok_or("missing value after --fixture-manifest")?,
                ));
            }
            "--write-fixture-manifest" => {
                write_fixture_manifest = Some(PathBuf::from(
                    values
                        .next()
                        .ok_or("missing value after --write-fixture-manifest")?,
                ));
            }
            "--rules" => {
                rules = Some(parse_rule_set(
                    &values.next().ok_or("missing value after --rules")?,
                )?);
            }
            "--no-source-closure" => {
                resolve_source_closure = false;
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unknown option `{arg}`").into());
            }
            _ => {
                if fixture.replace(arg).is_some() {
                    return Err("only one fixture may be provided".into());
                }
            }
        }
    }

    if fixture.is_none() && write_fixture_manifest.is_none() {
        return Err("missing fixture name".into());
    }
    Ok(Some(Args {
        fixture,
        iterations,
        corpus_root,
        fixture_manifest,
        write_fixture_manifest,
        rules,
        resolve_source_closure,
    }))
}

fn print_usage() {
    eprintln!(
        "Usage: large_corpus_profile <fixture> [--iterations N] [--root PATH] [--fixture-manifest PATH] [--rules SELECTORS] [--no-source-closure]"
    );
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  large_corpus_profile xwmx__nb__nb --iterations 10");
    eprintln!("  large_corpus_profile xwmx__nb__nb --rules C063");
    eprintln!("  large_corpus_profile --write-fixture-manifest /tmp/large-corpus.tsv");
}

fn parse_positive_usize(name: &str, value: Option<String>) -> Result<usize> {
    let value = value.ok_or_else(|| format!("missing value after {name}"))?;
    let parsed = value
        .parse::<usize>()
        .map_err(|err| format!("invalid {name} value `{value}`: {err}"))?;
    if parsed == 0 {
        return Err(format!("{name} must be greater than zero").into());
    }
    Ok(parsed)
}

fn parse_rule_set(value: &str) -> Result<RuleSet> {
    let mut rules = RuleSet::EMPTY;
    for selector in value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let selector = RuleSelector::from_str(selector)?;
        rules = rules.union(&selector.into_rule_set());
    }
    Ok(rules)
}

fn build_linter_settings(rules: Option<RuleSet>, resolve_source_closure: bool) -> LinterSettings {
    let settings = rules.map_or_else(LinterSettings::default, |rules| {
        LinterSettings::for_rules(rules.iter())
    });
    settings
        .with_c063_report_unreached_nested_definitions(true)
        .with_resolve_source_closure(resolve_source_closure)
}

fn run_large_corpus_fixture(
    fixture: &LargeCorpusFixture,
    base_settings: &LinterSettings,
    resolver: &LargeCorpusPathResolver,
) -> Result<usize> {
    let source = fs::read_to_string(&fixture.path)
        .map_err(|err| format!("failed to read {}: {err}", fixture.path.display()))?;
    let settings = base_settings
        .clone()
        .with_shell(ShellDialect::from_name(&fixture.shell))
        .with_analyzed_paths([fixture.path.clone()]);
    let parsed = Parser::with_dialect(&source, shuck_parser::ShellDialect::Bash).parse();
    let indexer = Indexer::new(&source, &parsed);
    let shellcheck_map = ShellCheckCodeMap::default();
    let diagnostics = lint_file_at_path_with_resolver_and_parse_result_with_comment_directives(
        &parsed,
        &source,
        &indexer,
        &settings,
        &shellcheck_map,
        Some(&fixture.path),
        Some(resolver),
    );

    Ok(diagnostics.len())
}

fn find_fixture<'a>(
    fixtures: &'a [LargeCorpusFixture],
    requested: &str,
) -> Option<&'a LargeCorpusFixture> {
    let requested_path = Path::new(requested);
    fixtures
        .iter()
        .find(|fixture| fixture.cache_rel_path == requested_path)
        .or_else(|| {
            fixtures
                .iter()
                .find(|fixture| fixture.cache_rel_path_key() == requested)
        })
        .or_else(|| {
            fixtures.iter().find(|fixture| {
                fixture.path.file_name().and_then(|name| name.to_str()) == Some(requested)
            })
        })
}

fn resolve_large_corpus_root(root_hint: Option<&Path>) -> Option<PathBuf> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let Some(repo_root) = manifest_dir.ancestors().nth(2) else {
        unreachable!("crates/shuck-benchmark should live two levels below repo root");
    };

    let env_hint = env::var(LARGE_CORPUS_ROOT_ENV)
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let candidates = root_hint
        .map(|hint| vec![hint.to_path_buf()])
        .or_else(|| env_hint.map(|hint| vec![hint]))
        .unwrap_or_else(|| {
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

fn collect_fixtures(corpus_dir: &Path) -> Vec<LargeCorpusFixture> {
    let scripts_dir = corpus_dir.join("scripts");
    let mut fixtures = Vec::new();

    for entry in walkdir::WalkDir::new(&scripts_dir)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            name != ".shuck_cache" && name != ".shellck_cache"
        })
    {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path().to_path_buf();
        if path_is_statically_ignored_large_corpus_fixture(&path)
            || path_is_sample_file(&path)
            || path_is_fish_file(&path)
            || path_is_patch_file(&path)
            || path_is_appledouble_file(&path)
            || path_is_guess_file(&path)
            || path_is_config_sub_file(&path)
        {
            continue;
        }

        let Ok(source) = fs::read(&path) else {
            continue;
        };
        let cache_rel_path = path
            .strip_prefix(&scripts_dir)
            .unwrap_or(path.as_path())
            .to_path_buf();
        let canonical_path = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        let shell = resolve_shell(&path, &source);

        fixtures.push(LargeCorpusFixture {
            path,
            canonical_path,
            cache_rel_path,
            shell,
        });
    }

    fixtures.sort_by(|left, right| left.path.cmp(&right.path));
    fixtures
}

fn write_fixture_manifest(path: &Path, fixtures: &[LargeCorpusFixture]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut contents = String::new();
    for fixture in fixtures {
        contents.push_str(&fixture.path.to_string_lossy());
        contents.push('\t');
        contents.push_str(&fixture.canonical_path.to_string_lossy());
        contents.push('\t');
        contents.push_str(&fixture.cache_rel_path.to_string_lossy());
        contents.push('\t');
        contents.push_str(&fixture.shell);
        contents.push('\n');
    }
    fs::write(path, contents)?;
    Ok(())
}

fn read_fixture_manifest(path: &Path) -> Result<Vec<LargeCorpusFixture>> {
    let contents = fs::read_to_string(path)
        .map_err(|err| format!("failed to read fixture manifest {}: {err}", path.display()))?;
    let mut fixtures = Vec::new();
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let Some(path) = parts.next() else {
            continue;
        };
        let Some(canonical_path) = parts.next() else {
            return Err(format!(
                "fixture manifest line {} is missing canonical path",
                index + 1
            )
            .into());
        };
        let Some(cache_rel_path) = parts.next() else {
            return Err(
                format!("fixture manifest line {} is missing cache path", index + 1).into(),
            );
        };
        let Some(shell) = parts.next() else {
            return Err(format!("fixture manifest line {} is missing shell", index + 1).into());
        };
        if parts.next().is_some() {
            return Err(format!("fixture manifest line {} has too many fields", index + 1).into());
        }
        fixtures.push(LargeCorpusFixture {
            path: PathBuf::from(path),
            canonical_path: PathBuf::from(canonical_path),
            cache_rel_path: PathBuf::from(cache_rel_path),
            shell: shell.to_owned(),
        });
    }
    Ok(fixtures)
}

fn resolve_shell(path: &Path, src: &[u8]) -> String {
    let source = String::from_utf8_lossy(src);
    let source = source.strip_prefix('\u{feff}').unwrap_or(source.as_ref());
    let trimmed_first_line = source
        .lines()
        .next()
        .map(|line| line.trim_start().to_ascii_lowercase())
        .unwrap_or_default();

    if trimmed_first_line.starts_with("#compdef") || trimmed_first_line.starts_with("#autoload") {
        return "zsh".into();
    }
    if path_has_large_corpus_static_zsh_override(path) {
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

fn normalize_cache_rel_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => Some(part.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn path_is_statically_ignored_large_corpus_fixture(path: &Path) -> bool {
    path_matches_large_corpus_suffix(path, LARGE_CORPUS_STATIC_IGNORE_SUFFIXES)
}

fn path_has_large_corpus_static_zsh_override(path: &Path) -> bool {
    path_matches_large_corpus_suffix(path, LARGE_CORPUS_STATIC_ZSH_OVERRIDE_SUFFIXES)
}

fn path_matches_large_corpus_suffix(path: &Path, suffixes: &[&str]) -> bool {
    let path = path.to_string_lossy();
    suffixes.iter().any(|suffix| path.ends_with(suffix))
}

fn path_is_sample_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".sample"))
}

fn path_is_patch_file(path: &Path) -> bool {
    let path = path.to_string_lossy().to_lowercase();
    path.ends_with(".patch") || path.ends_with(".diff") || path.ends_with(".dpatch")
}

fn path_is_fish_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("fish"))
}

fn path_is_appledouble_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("._"))
}

fn path_is_guess_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".guess"))
}

fn path_is_config_sub_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with("config.sub"))
}
