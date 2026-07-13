use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use shuck_config::{
    ConfigArguments, resolve_project_root_for_file, resolve_project_root_for_input,
};
use shuck_linter::{Applicability, LinterSettings, ShellCheckCodeMap, ShellDialect};
use shuck_parser::{Error as ParseError, parser::Parser};

use super::CheckReport;
use super::analyze::collect_lint_diagnostics;
use super::display::{display_lint_diagnostics_for_file, display_parse_error};
use super::settings::resolve_project_check_settings;
use super::source_resolver::NativeSourceResolver;
use crate::ExitStatus;
use crate::args::{CheckCommand, CheckOutputFormatArg};
use crate::commands::check_output::print_report_to;
use crate::discover::{DiscoveredFile, FileKind, ProjectRoot, normalize_path};
use crate::stdin::read_from_stdin;

pub(super) fn is_stdin(args: &CheckCommand) -> Result<bool> {
    if args.stdin_filename.is_some() {
        if args.paths.len() > 1 {
            return Err(anyhow!("cannot check multiple inputs together with stdin"));
        }
        if let Some(path) = args
            .paths
            .first()
            .filter(|path| path.as_path() != Path::new("-"))
        {
            return Err(anyhow!(
                "cannot check path `{}` together with --stdin-filename",
                path.display()
            ));
        }
        return Ok(true);
    }

    if args.paths.iter().any(|path| path == Path::new("-")) {
        if args.paths.len() != 1 {
            return Err(anyhow!("cannot check stdin together with filesystem paths"));
        }
        return Ok(true);
    }

    Ok(false)
}

pub(super) fn check_stdin(
    args: &CheckCommand,
    config_arguments: &ConfigArguments,
    cwd: &Path,
) -> Result<ExitStatus> {
    if args.watch {
        return Err(anyhow!("--watch cannot be used when checking stdin"));
    }
    if args.add_ignore.is_some() {
        return Err(anyhow!("--add-ignore cannot be used when checking stdin"));
    }

    let source = Arc::<str>::from(read_from_stdin()?);
    let file = stdin_file(args.stdin_filename.as_deref(), cwd, config_arguments)?;
    let settings = resolve_project_check_settings(
        &file.project_root,
        config_arguments,
        &args.rule_selection,
        &args.zsh_plugin_resolution,
    )?;
    let shell = settings
        .per_file_shell
        .shell_for_path(&file.absolute_path)
        .unwrap_or_else(|| ShellDialect::infer(&source, Some(&file.absolute_path)));
    let linter_settings = settings
        .linter_settings
        .clone()
        .with_analyzed_path_set(LinterSettings::analyzed_path_set([file
            .absolute_path
            .clone()]))
        .with_shell(shell);
    let shellcheck_map = ShellCheckCodeMap::default();
    // Relative `[lint] source-paths` resolve against the project root, matching
    // the file-based check path. Stdin only imports symbols from hinted
    // targets; it never lints them.
    let source_resolver = NativeSourceResolver::new(
        file.project_root.canonical_root.clone(),
        settings.source_paths.clone(),
    );
    let closure_resolver: Option<&(dyn shuck_semantic::SourcePathResolver + Send + Sync)> =
        source_resolver.has_roots().then_some(&source_resolver);
    let applicability = requested_fix_applicability(args);
    let include_source = matches!(args.output_format, CheckOutputFormatArg::Full);

    let mut checked_source = source;
    let mut parse_result = Parser::with_dialect(&checked_source, shell.parser_dialect()).parse();
    let mut analysis = collect_lint_diagnostics(
        &checked_source,
        &parse_result,
        &linter_settings,
        Some(settings.zsh_plugins.as_ref()),
        closure_resolver,
        &shellcheck_map,
        &file.absolute_path,
    );
    let mut fixes_applied = 0;

    if let Some(applicability) = applicability {
        let fixable = analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| settings.fixable_rules.contains(diagnostic.rule))
            .cloned()
            .collect::<Vec<_>>();
        let applied = shuck_linter::apply_fixes(&checked_source, &fixable, applicability);
        fixes_applied = applied.fixes_applied;
        checked_source = Arc::<str>::from(applied.code);
        parse_result = Parser::with_dialect(&checked_source, shell.parser_dialect()).parse();
        analysis = collect_lint_diagnostics(
            &checked_source,
            &parse_result,
            &linter_settings,
            Some(settings.zsh_plugins.as_ref()),
            closure_resolver,
            &shellcheck_map,
            &file.absolute_path,
        );
    }

    let parse_failed = parse_result.is_err();
    let diagnostics = if parse_failed && analysis.diagnostics.is_empty() {
        let ParseError::Parse {
            message,
            line,
            column,
        } = parse_result.strict_error();
        vec![display_parse_error(
            &file.display_path,
            &file.relative_path,
            &file.absolute_path,
            line,
            column,
            message,
            include_source.then_some(checked_source.clone()),
        )]
    } else {
        display_lint_diagnostics_for_file(
            &file,
            &checked_source,
            &analysis.diagnostics,
            include_source,
        )
    };
    let report = CheckReport {
        diagnostics,
        fixes_applied,
        parse_failed,
        dependency_paths: analysis.semantic.imported_dependency_paths().to_vec(),
        ..CheckReport::default()
    };

    if applicability.is_some() {
        let mut stdout = BufWriter::new(io::stdout().lock());
        stdout.write_all(checked_source.as_bytes())?;
        stdout.flush()?;

        let mut stderr = BufWriter::new(io::stderr().lock());
        print_report_to(
            &mut stderr,
            &report.diagnostics,
            args.output_format,
            colored::control::SHOULD_COLORIZE.should_colorize(),
        )?;
        if fixes_applied > 0 && is_human_readable(args.output_format) {
            writeln!(
                stderr,
                "Applied {fixes_applied} fix{}.",
                if fixes_applied == 1 { "" } else { "es" }
            )?;
        }
    } else {
        let mut stdout = BufWriter::new(io::stdout().lock());
        print_report_to(
            &mut stdout,
            &report.diagnostics,
            args.output_format,
            colored::control::SHOULD_COLORIZE.should_colorize(),
        )?;
    }

    Ok(report.exit_status(args.exit_zero, args.exit_non_zero_on_fix))
}

fn requested_fix_applicability(args: &CheckCommand) -> Option<Applicability> {
    if args.unsafe_fixes {
        Some(Applicability::Unsafe)
    } else if args.fix {
        Some(Applicability::Safe)
    } else {
        None
    }
}

fn is_human_readable(output_format: CheckOutputFormatArg) -> bool {
    matches!(
        output_format,
        CheckOutputFormatArg::Concise | CheckOutputFormatArg::Full | CheckOutputFormatArg::Grouped
    )
}

fn stdin_file(
    filename: Option<&Path>,
    cwd: &Path,
    config_arguments: &ConfigArguments,
) -> Result<DiscoveredFile> {
    let display_path = filename.unwrap_or_else(|| Path::new("-")).to_path_buf();
    let absolute_path = filename
        .map(|path| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            }
        })
        .unwrap_or_else(|| cwd.join("<stdin>"));
    let absolute_path = normalize_path(&absolute_path);
    let storage_root = match filename {
        Some(_) => {
            resolve_project_root_for_file(&absolute_path, cwd, config_arguments.use_config_roots())?
        }
        None => resolve_project_root_for_input(cwd, config_arguments.use_config_roots())?,
    };
    let canonical_root = std::fs::canonicalize(&storage_root)?;
    let relative_path = absolute_path
        .strip_prefix(&canonical_root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| display_path.clone());

    Ok(DiscoveredFile {
        display_path,
        absolute_path,
        relative_path,
        project_root: ProjectRoot {
            storage_root,
            canonical_root,
        },
        kind: FileKind::Shell,
    })
}
