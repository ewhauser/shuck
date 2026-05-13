use crate::facts::EnvPrefixExpansionFixFact;
use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct EnvPrefixExpansionOnly;

impl Violation for EnvPrefixExpansionOnly {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::EnvPrefixExpansionOnly
    }

    fn message(&self) -> String {
        "this same-command expansion still sees the earlier shell value".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("move the prefix assignment before the command".to_owned())
    }
}

pub fn env_prefix_expansion_only(checker: &mut Checker) {
    let source = checker.source();
    let facts = checker
        .facts()
        .env_prefix_expansion_fix_facts()
        .iter()
        .filter_map(|fact| {
            env_prefix_expansion_fix(fact, source).map(|fix| (fact.diagnostic_span(), fix))
        })
        .collect::<Vec<_>>();

    for (span, fix) in facts {
        checker
            .report_diagnostic_dedup(Diagnostic::new(EnvPrefixExpansionOnly, span).with_fix(fix));
    }

    checker.report_fact_slice_dedup(
        |facts| facts.env_prefix_expansion_scope_spans(),
        || EnvPrefixExpansionOnly,
    );
}

fn env_prefix_expansion_fix(fact: &EnvPrefixExpansionFixFact, source: &str) -> Option<Fix> {
    let indent = line_indent_before_offset(source, fact.delete_span().start.offset)?;
    let mut replacement = String::new();
    for assignment_span in fact.assignment_spans() {
        if !replacement.is_empty() {
            replacement.push_str(indent);
        }
        replacement.push_str(assignment_span.slice(source));
        replacement.push('\n');
    }
    replacement.push_str(indent);

    Some(Fix::unsafe_edit(Edit::replacement(
        replacement,
        fact.delete_span(),
    )))
}

fn line_indent_before_offset(source: &str, offset: usize) -> Option<&str> {
    let line_start = source[..offset]
        .rfind('\n')
        .map_or(0, |newline| newline + '\n'.len_utf8());
    let indent = source.get(line_start..offset)?;
    indent
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t'))
        .then_some(indent)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_later_expansions_that_cannot_see_prefix_assignments() {
        let source = "\
#!/bin/bash
CFLAGS=\"${SLKCFLAGS}\" ./configure --with-optmizer=${CFLAGS}
PATH=/tmp \"$PATH\"/bin/tool
A=1 B=\"$A\" C=\"$B\" cmd
foo=1 export \"$foo\"
foo=1 bar[$foo]=x cmd
FOO=tmp cmd >\"$FOO\"
COUNTDOWN=$[ $COUNTDOWN - 1 ] echo \"$COUNTDOWN\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EnvPrefixExpansionOnly),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${CFLAGS}",
                "$PATH",
                "$A",
                "$B",
                "$foo",
                "$foo",
                "$FOO",
                "$COUNTDOWN"
            ]
        );
    }

    #[test]
    fn ignores_nested_commands_and_assignment_only_forms() {
        let source = "\
#!/bin/bash
foo=1 echo hi
foo=\"$foo\" cmd
foo=1 cmd \"$(printf %s \"$foo\")\"
foo=1 foo=2 cmd
foo=1 bar=\"$foo\"
COUNTDOWN=$[ $COUNTDOWN - 1 ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EnvPrefixExpansionOnly),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_by_lifting_prefix_assignments() {
        let source = "\
#!/bin/bash
CFLAGS=\"${SLKCFLAGS}\" ./configure --with-optmizer=${CFLAGS}
  A=1 B=\"$A\" C=\"$B\" cmd
COUNTDOWN=$[ $COUNTDOWN - 1 ] echo \"$COUNTDOWN\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::EnvPrefixExpansionOnly),
            Applicability::Unsafe,
        );

        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
CFLAGS=\"${SLKCFLAGS}\"
./configure --with-optmizer=${CFLAGS}
  A=1
  B=\"$A\"
  C=\"$B\"
  cmd
COUNTDOWN=$[ $COUNTDOWN - 1 ]
echo \"$COUNTDOWN\"
"
        );
        assert_eq!(result.fixes_applied, 3);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_prefix_assignments_unchanged() {
        let source = "\
#!/bin/bash
A=1 B=\"$A\" cmd
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::EnvPrefixExpansionOnly),
            Applicability::Safe,
        );

        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixes_applied, 0);
    }

    #[test]
    fn skips_fix_for_inline_command_contexts() {
        let source = "\
#!/bin/bash
if A=1 B=\"$A\" cmd; then
  :
fi
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::EnvPrefixExpansionOnly),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.diagnostics.len(), 1);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C132.sh").as_path(),
            &LinterSettings::for_rule(Rule::EnvPrefixExpansionOnly),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C132_fix_C132.sh", result);
        Ok(())
    }
}
