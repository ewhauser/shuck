use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use shuck_indexer::Indexer;
use shuck_linter::{
    Checker, ExpansionContext, RuleSet, ShellDialect, WordFactContext, classify_file_context,
    static_word_text,
};
use shuck_parser::parser::Parser;
use shuck_parser::{ShellDialect as ParseShellDialect, ShellProfile};
use shuck_semantic::{OptionValue, SemanticBuildOptions, SemanticModel, ZshOptionState};
use tempfile::tempdir;

const OPTION_PROBE_PREFIX: &str = "__SHUCK_OPTIONS__";
const FIELD_PROBE_PREFIX: &str = "__SHUCK_FIELDS__";

#[derive(Debug, Deserialize)]
struct Fixture {
    name: String,
    source: String,
    #[serde(default)]
    files: Vec<FixtureFile>,
    #[serde(default)]
    option_probes: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    field_probes: BTreeMap<String, FieldProbeExpectation>,
}

#[derive(Debug, Deserialize)]
struct FixtureFile {
    path: String,
    #[serde(default)]
    contents: String,
}

#[derive(Debug, Deserialize)]
struct FieldProbeExpectation {
    count: usize,
    #[serde(default)]
    values: Vec<String>,
    #[serde(default)]
    shuck: Option<ShuckFieldExpectation>,
}

#[derive(Debug, Deserialize)]
struct ShuckFieldExpectation {
    multi: bool,
    field_splitting: bool,
    pathname_matching: bool,
}

#[derive(Debug, Default)]
struct ProbeRun {
    option_probes: BTreeMap<String, BTreeMap<String, String>>,
    field_probes: BTreeMap<String, FieldObservation>,
}

#[derive(Debug, Default)]
struct FieldObservation {
    count: usize,
    values: Vec<String>,
}

#[test]
fn zsh_option_state_fixtures_match_black_box_behavior() -> Result<()> {
    if !zsh_is_available()? {
        eprintln!("zsh option-state black-box fixtures skipped (`zsh` is unavailable)");
        return Ok(());
    }

    for fixture_path in fixture_paths()? {
        let fixture = load_fixture(&fixture_path)?;
        run_fixture(&fixture).with_context(|| format!("fixture {}", fixture.name))?;
    }

    Ok(())
}

fn zsh_is_available() -> Result<bool> {
    match Command::new("zsh")
        .args(["-fc", "emulate -R zsh; print -r -- ok"])
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                Ok(true)
            } else {
                bail!(
                    "`zsh` was found but could not execute fixtures successfully: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).context("failed to probe `zsh` availability"),
    }
}

fn fixture_paths() -> Result<Vec<PathBuf>> {
    let mut paths = fs::read_dir(fixture_dir())
        .with_context(|| format!("failed to read {}", fixture_dir().display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .context("failed to enumerate zsh option-state fixtures")?;
    paths.retain(|path| path.extension().and_then(|ext| ext.to_str()) == Some("toml"));
    paths.sort();
    Ok(paths)
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("testdata")
        .join("zsh-option-state")
}

fn load_fixture(path: &Path) -> Result<Fixture> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

fn run_fixture(fixture: &Fixture) -> Result<()> {
    let observed = run_fixture_in_zsh(fixture)?;
    let profile = ShellProfile::native(ParseShellDialect::Zsh);
    let parsed = Parser::with_profile(&fixture.source, profile.clone()).parse();
    if parsed.is_err() {
        return Err(anyhow::anyhow!("parse error: {}", parsed.strict_error()));
    }
    let indexer = Indexer::new(&fixture.source, &parsed);
    let semantic = SemanticModel::build_with_options(
        &parsed.file,
        &fixture.source,
        &indexer,
        SemanticBuildOptions {
            shell_profile: Some(profile),
            ..SemanticBuildOptions::default()
        },
    );
    let file_context = classify_file_context(&fixture.source, None, ShellDialect::Zsh);
    let rules = RuleSet::EMPTY;
    let checker = Checker::new(
        &parsed.file,
        &fixture.source,
        &semantic,
        &indexer,
        &rules,
        ShellDialect::Zsh,
        &file_context,
    );

    for (probe_id, expected) in &fixture.option_probes {
        let actual = observed
            .option_probes
            .get(probe_id)
            .with_context(|| format!("missing zsh option probe `{probe_id}`"))?;
        assert_eq!(
            actual, expected,
            "fixture `{}` probe `{probe_id}` zsh option output mismatch",
            fixture.name
        );

        let fact = find_probe_command(&checker, "__shuck_probe_options", probe_id)?;
        let predicted = expected
            .keys()
            .map(|name| {
                let options = fact
                    .zsh_options()
                    .with_context(|| format!("missing zsh option state for `{probe_id}`"))?;
                Ok((
                    name.clone(),
                    option_value_string(lookup_option(options, name)?),
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        assert_eq!(
            predicted, *expected,
            "fixture `{}` probe `{probe_id}` shuck option state mismatch",
            fixture.name
        );
    }

    for (probe_id, expected) in &fixture.field_probes {
        let actual = observed
            .field_probes
            .get(probe_id)
            .with_context(|| format!("missing zsh field probe `{probe_id}`"))?;
        assert_eq!(
            actual.count, expected.count,
            "fixture `{}` probe `{probe_id}` zsh field-count mismatch",
            fixture.name
        );
        assert_eq!(
            actual.values, expected.values,
            "fixture `{}` probe `{probe_id}` zsh field-value mismatch",
            fixture.name
        );

        let Some(shuck) = &expected.shuck else {
            continue;
        };

        let fact = find_probe_command(&checker, "__shuck_probe_fields", probe_id)?;
        let target = fact.body_args().get(1).with_context(|| {
            format!(
                "fixture `{}` probe `{probe_id}` did not expose a field target word",
                fixture.name
            )
        })?;
        let word_fact = checker
            .facts()
            .word_fact(
                target.span,
                WordFactContext::Expansion(ExpansionContext::CommandArgument),
            )
            .with_context(|| {
                format!(
                    "fixture `{}` probe `{probe_id}` missing command-argument word fact",
                    fixture.name
                )
            })?;
        let analysis = word_fact.analysis();
        assert_eq!(
            analysis.can_expand_to_multiple_fields, shuck.multi,
            "fixture `{}` probe `{probe_id}` shuck multi-field mismatch",
            fixture.name
        );
        assert_eq!(
            analysis.hazards.field_splitting, shuck.field_splitting,
            "fixture `{}` probe `{probe_id}` shuck field-splitting mismatch",
            fixture.name
        );
        assert_eq!(
            analysis.hazards.pathname_matching, shuck.pathname_matching,
            "fixture `{}` probe `{probe_id}` shuck pathname-matching mismatch",
            fixture.name
        );
    }

    Ok(())
}

fn run_fixture_in_zsh(fixture: &Fixture) -> Result<ProbeRun> {
    let tempdir = tempdir().context("failed to create temp dir for zsh fixture")?;
    for file in &fixture.files {
        let path = tempdir.path().join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&path, &file.contents)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }

    let script = format!(
        r#"emulate -R zsh
__shuck_probe_options() {{
  local id=$1
  shift
  printf '%s\t%s' '{OPTION_PROBE_PREFIX}' "$id"
  local opt
  for opt in "$@"; do
    if [[ -o "$opt" ]]; then
      printf '\t%s=on' "$opt"
    else
      printf '\t%s=off' "$opt"
    fi
  done
  printf '\n'
}}
__shuck_probe_fields() {{
  local id=$1
  shift
  printf '%s\t%s\t%s' '{FIELD_PROBE_PREFIX}' "$id" "$#"
  local arg
  for arg in "$@"; do
    printf '\t%s' "$arg"
  done
  printf '\n'
}}
{}
"#,
        fixture.source
    );

    let output = Command::new("zsh")
        .args(["-fc", &script])
        .current_dir(tempdir.path())
        .output()
        .context("failed to execute zsh fixture")?;
    if !output.status.success() {
        bail!(
            "zsh fixture failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    parse_probe_output(
        &String::from_utf8(output.stdout).context("zsh fixture stdout was not utf-8")?,
    )
}

fn parse_probe_output(stdout: &str) -> Result<ProbeRun> {
    let mut run = ProbeRun::default();
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        match fields.as_slice() {
            [OPTION_PROBE_PREFIX, probe_id, rest @ ..] => {
                let options = rest
                    .iter()
                    .map(|entry| {
                        let (name, value) = entry.split_once('=').with_context(|| {
                            format!("invalid option probe entry `{entry}` in `{line}`")
                        })?;
                        Ok((name.to_owned(), value.to_owned()))
                    })
                    .collect::<Result<BTreeMap<_, _>>>()?;
                run.option_probes.insert((*probe_id).to_owned(), options);
            }
            [FIELD_PROBE_PREFIX, probe_id, count, values @ ..] => {
                run.field_probes.insert(
                    (*probe_id).to_owned(),
                    FieldObservation {
                        count: count.parse().with_context(|| {
                            format!("invalid field count `{count}` in `{line}`")
                        })?,
                        values: values.iter().map(|value| (*value).to_owned()).collect(),
                    },
                );
            }
            _ => {}
        }
    }
    Ok(run)
}

fn find_probe_command<'a>(
    checker: &'a Checker<'_>,
    command_name: &str,
    probe_id: &str,
) -> Result<&'a shuck_linter::CommandFact<'a>> {
    checker
        .facts()
        .structural_commands()
        .find(|fact| {
            fact.effective_name_is(command_name)
                && fact
                    .body_args()
                    .first()
                    .and_then(|word| static_word_text(word, checker.source()))
                    .as_deref()
                    == Some(probe_id)
        })
        .with_context(|| format!("missing `{command_name}` probe command for `{probe_id}`"))
}

fn lookup_option<'a>(options: &'a ZshOptionState, name: &str) -> Result<&'a OptionValue> {
    match name {
        "shwordsplit" => Ok(&options.sh_word_split),
        "globsubst" => Ok(&options.glob_subst),
        "rcexpandparam" => Ok(&options.rc_expand_param),
        "glob" => Ok(&options.glob),
        "nomatch" => Ok(&options.nomatch),
        "nullglob" => Ok(&options.null_glob),
        "cshnullglob" => Ok(&options.csh_null_glob),
        "extendedglob" => Ok(&options.extended_glob),
        "kshglob" => Ok(&options.ksh_glob),
        "shglob" => Ok(&options.sh_glob),
        "bareglobqual" => Ok(&options.bare_glob_qual),
        "globdots" => Ok(&options.glob_dots),
        "equals" => Ok(&options.equals),
        "magicequalsubst" => Ok(&options.magic_equal_subst),
        "shfileexpansion" => Ok(&options.sh_file_expansion),
        "globassign" => Ok(&options.glob_assign),
        "ignorebraces" => Ok(&options.ignore_braces),
        "ignoreclosebraces" => Ok(&options.ignore_close_braces),
        "braceccl" => Ok(&options.brace_ccl),
        "ksharrays" => Ok(&options.ksh_arrays),
        "kshzerosubscript" => Ok(&options.ksh_zero_subscript),
        "shortloops" => Ok(&options.short_loops),
        "shortrepeat" => Ok(&options.short_repeat),
        "rcquotes" => Ok(&options.rc_quotes),
        "interactivecomments" => Ok(&options.interactive_comments),
        "cbases" => Ok(&options.c_bases),
        "octalzeroes" => Ok(&options.octal_zeroes),
        _ => bail!("unsupported fixture option name `{name}`"),
    }
}

fn option_value_string(value: &OptionValue) -> String {
    match value {
        OptionValue::On => "on",
        OptionValue::Off => "off",
        OptionValue::Unknown => "unknown",
    }
    .to_owned()
}
