use std::path::Path;

use anyhow::{Result, anyhow};

use crate::ExitStatus;
use crate::args::CheckCommand;
use crate::cache::resolve_cache_root;
use crate::commands::check_output::{DisplayedDiagnostic, DisplayedDiagnosticKind};
use crate::config::ConfigArguments;

mod add_ignore;
mod analyze;
mod cache;
mod display;
mod embedded;
mod run;
mod settings;
mod watch;

use add_ignore::run_add_ignore_with_cwd;
use display::{print_diagnostics, print_report};
pub(crate) use run::benchmark_check_paths;
use run::run_check_with_cwd;
use watch::watch_check;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CheckReport {
    diagnostics: Vec<DisplayedDiagnostic>,
    cache_hits: usize,
    cache_misses: usize,
    fixes_applied: usize,
    parse_failed: bool,
}

impl CheckReport {
    fn exit_status(&self, exit_zero: bool, exit_non_zero_on_fix: bool) -> ExitStatus {
        if exit_non_zero_on_fix && self.fixes_applied > 0 {
            return ExitStatus::Failure;
        }
        if self.parse_failed {
            return ExitStatus::Failure;
        }
        diagnostics_exit_status(&self.diagnostics, exit_zero)
    }
}

fn diagnostics_exit_status(diagnostics: &[DisplayedDiagnostic], exit_zero: bool) -> ExitStatus {
    let has_fatal = diagnostics.iter().any(|d| match &d.kind {
        DisplayedDiagnosticKind::ParseError => true,
        DisplayedDiagnosticKind::Lint { severity, .. } => severity == "error",
    });
    if has_fatal {
        return ExitStatus::Failure;
    }
    if diagnostics.is_empty() || exit_zero {
        ExitStatus::Success
    } else {
        ExitStatus::Failure
    }
}

pub(crate) fn check(
    args: CheckCommand,
    config_arguments: &ConfigArguments,
    cache_dir: Option<&Path>,
) -> Result<ExitStatus> {
    let cwd = std::env::current_dir()?;
    let cache_root = resolve_cache_root(&cwd, cache_dir)?;
    if args.watch {
        return watch_check(&args, config_arguments, &cwd, &cache_root);
    }

    if let Some(raw_reason) = args.add_ignore.as_deref() {
        if raw_reason.contains(['\n', '\r']) {
            return Err(anyhow!(
                "--add-ignore <reason> cannot contain newline characters"
            ));
        }

        let report = run_add_ignore_with_cwd(
            &args,
            config_arguments,
            &cwd,
            &cache_root,
            (!raw_reason.is_empty()).then_some(raw_reason),
        )?;
        if report.directives_added > 0 {
            let s = if report.directives_added == 1 {
                ""
            } else {
                "s"
            };
            eprintln!(
                "Added {} shuck ignore directive{s}.",
                report.directives_added
            );
        }
        print_diagnostics(&report.diagnostics, args.output_format)?;
        return Ok(report.exit_status(args.exit_zero));
    }

    let report = run_check_with_cwd(&args, config_arguments, &cwd, &cache_root)?;
    print_report(&report, args.output_format)?;
    Ok(report.exit_status(args.exit_zero, args.exit_non_zero_on_fix))
}

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::check::test_support::{
        lint_displayed_diagnostic, parse_displayed_diagnostic,
    };
    use crate::commands::check_output::DisplaySpan;

    #[test]
    fn exit_zero_suppresses_only_non_fatal_diagnostics() {
        let warning = lint_displayed_diagnostic(
            "warn.sh",
            DisplaySpan::point(1, 1),
            "lint",
            "C001",
            "warning",
        );
        let error_lint =
            lint_displayed_diagnostic("err.sh", DisplaySpan::point(1, 1), "lint", "C035", "error");
        let parse = parse_displayed_diagnostic("broken.sh", DisplaySpan::point(1, 1), "parse");

        let warning_only = CheckReport {
            diagnostics: vec![warning.clone()],
            ..CheckReport::default()
        };
        assert_eq!(warning_only.exit_status(false, false), ExitStatus::Failure);
        assert_eq!(warning_only.exit_status(true, false), ExitStatus::Success);

        let with_error_lint = CheckReport {
            diagnostics: vec![warning.clone(), error_lint],
            ..CheckReport::default()
        };
        assert_eq!(
            with_error_lint.exit_status(true, false),
            ExitStatus::Failure
        );

        let with_parse_error = CheckReport {
            diagnostics: vec![warning, parse],
            ..CheckReport::default()
        };
        assert_eq!(
            with_parse_error.exit_status(true, false),
            ExitStatus::Failure
        );
    }

    #[test]
    fn exit_non_zero_on_fix_fires_when_fixes_applied() {
        let report = CheckReport {
            fixes_applied: 1,
            ..CheckReport::default()
        };
        assert_eq!(report.exit_status(false, false), ExitStatus::Success);
        assert_eq!(report.exit_status(false, true), ExitStatus::Failure);
        assert_eq!(report.exit_status(true, true), ExitStatus::Failure);
    }
}
