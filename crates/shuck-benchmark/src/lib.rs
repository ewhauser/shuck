use std::path::{Path, PathBuf};
use std::sync::LazyLock;

#[cfg(feature = "parser-benchmarking")]
use shuck_parser::parser::ParserBenchmarkCounters;
use shuck_parser::parser::{ParseOutput, Parser};

/// Categorize fixtures by expected runtime so Criterion can spend
/// more time where it is useful without making the slowest cases drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestCaseSpeed {
    Fast,
    Normal,
    Slow,
}

impl TestCaseSpeed {
    pub fn sample_size(self) -> usize {
        match self {
            Self::Fast => 100,
            Self::Normal => 20,
            // Criterion enforces a minimum sample size of 10.
            Self::Slow => 10,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestFile {
    pub name: &'static str,
    pub source: &'static str,
    pub speed: TestCaseSpeed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestCase {
    pub name: &'static str,
    pub files: &'static [TestFile],
    pub speed: TestCaseSpeed,
}

impl TestCase {
    pub fn total_bytes(self) -> u64 {
        self.files.iter().map(|file| file.source.len() as u64).sum()
    }
}

fn fixture_source(bytes: &'static [u8]) -> &'static str {
    let source = std::str::from_utf8(bytes).expect("benchmark fixtures should be valid UTF-8");
    if source.contains('\r') {
        Box::leak(
            source
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .into_boxed_str(),
        )
    } else {
        source
    }
}

pub static TEST_FILES: LazyLock<Vec<TestFile>> = LazyLock::new(|| {
    vec![
        TestFile {
            name: "fzf-install",
            source: fixture_source(include_bytes!("../resources/files/fzf-install.sh")),
            speed: TestCaseSpeed::Fast,
        },
        TestFile {
            name: "homebrew-install",
            source: fixture_source(include_bytes!("../resources/files/homebrew-install.sh")),
            speed: TestCaseSpeed::Fast,
        },
        TestFile {
            name: "ruby-build",
            source: fixture_source(include_bytes!("../resources/files/ruby-build.sh")),
            speed: TestCaseSpeed::Normal,
        },
        TestFile {
            name: "pyenv-python-build",
            source: fixture_source(include_bytes!("../resources/files/pyenv-python-build.sh")),
            speed: TestCaseSpeed::Normal,
        },
        TestFile {
            name: "nvm",
            source: fixture_source(include_bytes!("../resources/files/nvm.sh")),
            speed: TestCaseSpeed::Slow,
        },
    ]
});

pub fn benchmark_cases() -> Vec<TestCase> {
    let mut cases = TEST_FILES
        .iter()
        .map(|file| TestCase {
            name: file.name,
            files: std::slice::from_ref(file),
            speed: file.speed,
        })
        .collect::<Vec<_>>();

    cases.push(TestCase {
        name: "all",
        files: TEST_FILES.as_slice(),
        speed: TestCaseSpeed::Slow,
    });

    cases
}

pub fn resources_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("resources")
}

fn parse_fixture_with<T, E>(
    source: &str,
    parse: impl FnOnce(Parser<'_>) -> std::result::Result<T, E>,
    recover: impl FnOnce(Parser<'_>) -> T,
) -> (T, bool) {
    match parse(Parser::new(source)) {
        Ok(output) => (output, false),
        Err(_) => (recover(Parser::new(source)), true),
    }
}

pub fn parse_fixture(source: &str) -> ParseOutput {
    parse_fixture_with(
        source,
        |parser| parser.parse(),
        |parser| ParseOutput {
            file: parser.parse_recovered().file,
        },
    )
    .0
}

#[cfg(feature = "parser-benchmarking")]
#[doc(hidden)]
pub struct CountedParseFixtureOutput {
    pub output: ParseOutput,
    pub counters: ParserBenchmarkCounters,
    pub recovered: bool,
}

#[cfg(feature = "parser-benchmarking")]
#[doc(hidden)]
pub fn parse_fixture_with_benchmark_counters(source: &str) -> CountedParseFixtureOutput {
    let ((output, counters), recovered) = parse_fixture_with(
        source,
        |parser| parser.parse_with_benchmark_counters(),
        |parser| {
            let (recovered, counters) = parser.parse_recovered_with_benchmark_counters();
            (
                ParseOutput {
                    file: recovered.file,
                },
                counters,
            )
        },
    );

    CountedParseFixtureOutput {
        output,
        counters,
        recovered,
    }
}

#[macro_export]
macro_rules! configure_benchmark_allocator {
    () => {
        #[cfg(not(target_os = "windows"))]
        #[global_allocator]
        static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    };
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "parser-benchmarking")]
    use super::{Parser, parse_fixture_with_benchmark_counters};
    use super::{TEST_FILES, benchmark_cases, parse_fixture, resources_dir};
    use serde::Deserialize;
    use shuck_formatter::{FormattedSource, ShellFormatOptions, format_file_ast, format_source};
    use shuck_indexer::Indexer;
    use shuck_linter::{
        LinterSettings, ShellCheckCodeMap, SuppressionIndex, first_statement_line, lint_file,
        parse_directives,
    };

    #[derive(Debug, Deserialize)]
    struct Manifest {
        fixtures: Vec<Fixture>,
    }

    #[derive(Debug, Deserialize)]
    struct Fixture {
        local_filename: String,
        byte_size: usize,
    }

    #[test]
    fn fixture_sources_match_manifest_sizes() {
        let manifest = serde_json::from_str::<Manifest>(include_str!("../resources/manifest.json"))
            .expect("benchmark fixture manifest should parse");

        assert_eq!(manifest.fixtures.len(), TEST_FILES.len());

        let fixture_sizes = manifest
            .fixtures
            .iter()
            .map(|fixture| (fixture.local_filename.as_str(), fixture.byte_size))
            .collect::<std::collections::BTreeMap<_, _>>();

        for test_file in TEST_FILES.iter() {
            let local_filename = format!("files/{}.sh", test_file.name);
            assert_eq!(
                fixture_sizes.get(local_filename.as_str()).copied(),
                Some(test_file.source.len()),
                "{}",
                test_file.name
            );
            assert!(!test_file.source.is_empty(), "{}", test_file.name);
        }
    }

    #[test]
    fn benchmark_cases_include_per_file_and_aggregate_cases() {
        let cases = benchmark_cases();

        assert_eq!(cases.len(), TEST_FILES.len() + 1);
        assert_eq!(cases.last().map(|case| case.name), Some("all"));
        assert_eq!(
            cases.last().map(|case| case.total_bytes()),
            Some(TEST_FILES.iter().map(|file| file.source.len() as u64).sum())
        );
    }

    #[test]
    fn resources_directory_exists() {
        assert!(resources_dir().is_dir());
    }

    #[test]
    fn benchmark_corpus_parses_in_best_effort_mode() {
        for file in TEST_FILES.iter() {
            let output = parse_fixture(file.source);
            assert!(
                !output.file.body.is_empty(),
                "{} should produce some parsed commands",
                file.name
            );
        }
    }

    #[test]
    fn benchmark_corpus_survives_lint_pipeline() {
        let settings = LinterSettings::default();
        let shellcheck_map = ShellCheckCodeMap::default();

        for file in TEST_FILES.iter() {
            let output = parse_fixture(file.source);
            let indexer = Indexer::new(file.source, &output);
            let directives =
                parse_directives(file.source, indexer.comment_index(), &shellcheck_map);
            let suppression_index = (!directives.is_empty()).then(|| {
                SuppressionIndex::new(
                    &directives,
                    &output.file,
                    first_statement_line(&output.file).unwrap_or(u32::MAX),
                )
            });
            let diagnostics = lint_file(
                &output.file,
                file.source,
                &indexer,
                &settings,
                suppression_index.as_ref(),
            );

            assert!(
                diagnostics.len() < usize::MAX,
                "{} should produce a finite diagnostic set",
                file.name
            );
        }
    }

    #[test]
    fn benchmark_corpus_survives_formatter_pipeline() {
        let options = ShellFormatOptions::default();

        for file in TEST_FILES.iter() {
            match format_source(file.source, None, &options) {
                Ok(FormattedSource::Unchanged) | Ok(FormattedSource::Formatted(_)) => {}
                Err(error) => panic!("{} should format successfully: {error}", file.name),
            }
        }
    }

    #[test]
    fn benchmark_corpus_survives_ast_formatter_pipeline() {
        let options = ShellFormatOptions::default();

        for file in TEST_FILES.iter() {
            let output = parse_fixture(file.source);
            match format_file_ast(file.source, output.file, None, &options) {
                Ok(FormattedSource::Unchanged) | Ok(FormattedSource::Formatted(_)) => {}
                Err(error) => panic!("{} should format from AST successfully: {error}", file.name),
            }
        }
    }

    #[cfg(feature = "parser-benchmarking")]
    #[test]
    fn counted_parse_fixture_matches_best_effort_parse_mode() {
        let file = TEST_FILES
            .iter()
            .find(|file| file.name == "nvm")
            .expect("nvm benchmark fixture should exist");

        let counted = parse_fixture_with_benchmark_counters(file.source);
        let uncounted_recovered = Parser::new(file.source).parse().is_err();

        assert_eq!(counted.recovered, uncounted_recovered);
        assert!(
            !counted.output.file.body.is_empty(),
            "counted parse should produce some parsed commands"
        );
        assert!(counted.counters.lexer_current_position_calls > 0);
        assert!(counted.counters.parser_set_current_spanned_calls > 0);
        assert!(counted.counters.parser_advance_raw_calls > 0);
    }
}
