use std::path::Path;

use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct NonAbsoluteShebang;

impl Violation for NonAbsoluteShebang {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::NonAbsoluteShebang
    }

    fn message(&self) -> String {
        "shebang should use an absolute path or `/usr/bin/env`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite the shebang to use `/usr/bin/env`".to_owned())
    }
}

pub fn non_absolute_shebang(checker: &mut Checker) {
    let source = checker.source();
    if let Some(span) = checker.facts().non_absolute_shebang_span() {
        let replacement = rewrite_shebang(span.slice(source));
        checker.report_diagnostic_dedup(
            crate::Diagnostic::new(NonAbsoluteShebang, span)
                .with_fix(Fix::unsafe_edit(Edit::replacement(replacement, span))),
        );
    }
}

fn rewrite_shebang(shebang_line: &str) -> String {
    let mut words = shebang_line
        .strip_prefix("#!")
        .map(str::split_whitespace)
        .into_iter()
        .flatten();

    let interpreter = words.next().unwrap_or_default();
    let interpreter = Path::new(interpreter)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(interpreter);

    let mut rewritten = String::from("#!/usr/bin/env");
    if interpreter != "env" {
        rewritten.push(' ');
        rewritten.push_str(interpreter);
    }

    for arg in words {
        rewritten.push(' ');
        rewritten.push_str(arg);
    }

    rewritten
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_non_absolute_shebangs() {
        let source = "#!bin/sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::NonAbsoluteShebang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].span.slice(source), "#!bin/sh");
    }

    #[test]
    fn ignores_absolute_and_env_shebangs() {
        for source in [
            "#!/bin/sh\n:\n",
            "#!/usr/bin/env sh\n:\n",
            "#! /bin/sh\n:\n",
        ] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::NonAbsoluteShebang));
            assert!(diagnostics.is_empty());
        }
    }

    #[test]
    fn ignores_non_absolute_shebang_when_shellcheck_shell_directive_is_present() {
        let source = "#!@TERMUX_PREFIX@/bin/sh\n# shellcheck shell=sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::NonAbsoluteShebang));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn exposes_unsafe_fix_metadata_for_reported_shebangs() {
        let source = "#!@PREFIX@/bin/sh -x\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::NonAbsoluteShebang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("rewrite the shebang to use `/usr/bin/env`")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_non_absolute_shebangs() {
        let source = "#!@PREFIX@/bin/sh -x\n:\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::NonAbsoluteShebang),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/usr/bin/env sh -x\n:\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn preserves_existing_env_interpreters_when_rewriting() {
        for (source, expected) in [
            ("#!env bash\n:\n", "#!/usr/bin/env bash\n:\n"),
            ("#!env -S bash -e\n:\n", "#!/usr/bin/env -S bash -e\n:\n"),
        ] {
            let result = test_snippet_with_fix(
                source,
                &LinterSettings::for_rule(Rule::NonAbsoluteShebang),
                Applicability::Unsafe,
            );

            assert_eq!(result.fixes_applied, 1);
            assert_eq!(result.fixed_source, expected);
            assert!(result.fixed_diagnostics.is_empty());
        }
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C060.sh").as_path(),
            &LinterSettings::for_rule(Rule::NonAbsoluteShebang),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C060_fix_C060.sh", result);
        Ok(())
    }
}
