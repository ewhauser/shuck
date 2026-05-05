use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation, WrapperKind};

pub struct GlobInFindSubstitution;

impl Violation for GlobInFindSubstitution {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::GlobInFindSubstitution
    }

    fn message(&self) -> String {
        "quote glob patterns passed to `find` so the shell does not expand them early".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("wrap the reported `find` pattern operand in double quotes".to_owned())
    }
}

pub fn glob_in_find_substitution(checker: &mut Checker) {
    let source = checker.source();
    let facts = checker.facts();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| {
            fact.wrappers()
                .iter()
                .all(|wrapper| matches!(wrapper, WrapperKind::FindExec | WrapperKind::FindExecDir))
        })
        .filter_map(|fact| fact.options().find())
        .flat_map(|find| find.glob_pattern_operand_spans().iter().copied())
        .map(|span| {
            let replacement = facts
                .any_word_fact(span)
                .expect("find pattern operand span should map to a word fact")
                .single_double_quoted_replacement(source);
            crate::Diagnostic::new(GlobInFindSubstitution, span)
                .with_fix(Fix::unsafe_edit(Edit::replacement(replacement, span)))
        })
        .collect::<Vec<_>>();

    for diagnostic in spans {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::test::{test_path_with_fix, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};
    use std::path::Path;

    #[test]
    fn reports_find_pattern_operands_that_can_glob_expand() {
        let source = "\
#!/bin/bash
find ./ -name *.jar
find ./ -name \"$prefix\"*.jar
find ./ -wholename */tmp/*
find ./ -name *.so -exec chmod 755 {} \\;
find ./ -name \\*.[15] -exec gzip -9 {} \\;
for f in $(find ./ -name *.cfg); do :; done
printf '%s\\n' \"$(find . -path */tmp/*)\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInFindSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "*.jar",
                "\"$prefix\"*.jar",
                "*/tmp/*",
                "*.so",
                "\\*.[15]",
                "*.cfg",
                "*/tmp/*"
            ]
        );
    }

    #[test]
    fn ignores_quoted_non_pattern_and_wrapped_find_operands() {
        let source = "\
#!/bin/bash
find ./ -name '*.jar'
find ./ -name \\*.tmp
find ./ -path \\*/tmp/\\*
find ./ -wholename \\*/tmp/\\*
find ./ -type f*
command find ./ -name *.jar
find ./ -name \"$pattern\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInFindSubstitution),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn attaches_unsafe_fix_metadata_for_find_pattern_operands() {
        let source = "#!/bin/bash\nfind ./ -name *.jar\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInFindSubstitution),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("wrap the reported `find` pattern operand in double quotes")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_find_pattern_operands_preserving_expansions() {
        let source = "\
#!/bin/bash
find ./ -name \"$prefix\"*.jar
printf '%s\\n' \"$(find . -path \"$prefix\"*/tmp/*)\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GlobInFindSubstitution),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
find ./ -name \"${prefix}*.jar\"
printf '%s\\n' \"$(find . -path \"${prefix}*/tmp/*\")\"
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_find_pattern_operands() {
        let source = "\
#!/bin/bash
find ./ -name *.jar
find ./ -wholename */tmp/*
find ./ -name \\*.[15] -exec gzip -9 {} \\;
for f in $(find ./ -name *.cfg); do :; done
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GlobInFindSubstitution),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 4);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
find ./ -name \"*.jar\"
find ./ -wholename \"*/tmp/*\"
find ./ -name \"*.[15]\" -exec gzip -9 {} \\;
for f in $(find ./ -name \"*.cfg\"); do :; done
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_quoted_find_patterns_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
find ./ -name '*.jar'
find ./ -name \"$pattern\"
command find ./ -name *.jar
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GlobInFindSubstitution),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn respects_zsh_glob_activation_for_find_pattern_operands() {
        let source = "\
#!/usr/bin/env zsh
setopt no_glob
find ./ -name *.jar
setopt glob extended_glob
find ./ -name foo^bar
find ./ -name foo~bar
find ./ -name foo~bar*
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInFindSubstitution)
                .with_shell(crate::ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foo^bar", "foo~bar*"]
        );
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C083.sh").as_path(),
            &LinterSettings::for_rule(Rule::GlobInFindSubstitution),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C083_fix_C083.sh", result);
        Ok(())
    }
}
