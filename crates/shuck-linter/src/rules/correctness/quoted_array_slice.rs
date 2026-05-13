use crate::{
    Checker, Diagnostic, Edit, ExpansionContext, Fix, FixAvailability, Rule, Violation,
    WordFactHostKind,
};

pub struct QuotedArraySlice;

impl Violation for QuotedArraySlice {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::QuotedArraySlice
    }

    fn message(&self) -> String {
        "all-elements array expansions collapse in scalar assignment values".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite the expansion as an intentional join".to_owned())
    }
}

pub fn quoted_array_slice(checker: &mut Checker) {
    let facts = checker.facts();
    let locator = checker.locator();
    let source = checker.source();
    let diagnostics = [
        ExpansionContext::AssignmentValue,
        ExpansionContext::DeclarationAssignmentValue,
    ]
    .into_iter()
    .flat_map(|context| facts.words().expansion_word_facts(context))
    .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
    .filter(|fact| !facts.words().is_compound_assignment_value_word(*fact))
    .filter(|fact| fact.has_direct_all_elements_array_expansion_in_source(locator))
    .map(|fact| {
        (
            fact.span(),
            intentional_join_fix(fact.direct_all_elements_array_expansion_spans(), source),
        )
    })
    .collect::<Vec<_>>();

    for (span, fix) in diagnostics {
        let diagnostic = Diagnostic::new(QuotedArraySlice, span);
        if let Some(fix) = fix {
            checker.report_diagnostic_dedup(diagnostic.with_fix(fix));
        } else {
            checker.report_diagnostic_dedup(diagnostic);
        }
    }
}

fn intentional_join_fix(expansion_spans: &[shuck_ast::Span], source: &str) -> Option<Fix> {
    expansion_spans
        .iter()
        .map(|span| {
            intentional_join_expansion(span.slice(source))
                .map(|replacement| Edit::replacement(replacement, *span))
        })
        .collect::<Option<Vec<_>>>()
        .filter(|edits| !edits.is_empty())
        .map(Fix::unsafe_edits)
}

fn intentional_join_expansion(raw: &str) -> Option<String> {
    if raw == "$@" {
        return Some("$*".to_owned());
    }

    if let Some(rest) = raw.strip_prefix("$@[*]") {
        return Some(format!("$*{rest}"));
    }

    if raw.starts_with("${@") {
        return Some(raw.replacen("${@", "${*", 1));
    }

    if raw.contains("[@]") {
        return Some(raw.replacen("[@]", "[*]", 1));
    }

    None
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect, assert_diagnostics_diff};

    #[test]
    fn reports_all_elements_array_expansions_in_scalar_bindings() {
        let source = "\
#!/bin/bash
x=\"$@\"
y=\"${@}\"
z=${@:5}
p=\"${arr[@]}\"
q=\"${arr[@]:-fallback}\"
r=\"${arr[@]@Q}\"
flags+=\" ${add_flags[@]}\"
targets[$key]=\"${items[@]}\"
CFLAGS+=\" ${add_flags[@]}\" make
escaped=\"\\\\$@\"
escaped_slice=\"\\\\${@:2}\"
declare declared=\"$@\"
readonly packed=${arr[@]}
f() { local nested=\"${@:3}\"; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedArraySlice));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"$@\"",
                "\"${@}\"",
                "${@:5}",
                "\"${arr[@]}\"",
                "\"${arr[@]:-fallback}\"",
                "\"${arr[@]@Q}\"",
                "\" ${add_flags[@]}\"",
                "\"${items[@]}\"",
                "\" ${add_flags[@]}\"",
                "\"\\\\$@\"",
                "\"\\\\${@:2}\"",
                "\"$@\"",
                "${arr[@]}",
                "\"${@:3}\"",
            ]
        );
    }

    #[test]
    fn reports_quoted_array_slice_assignments_into_scalar_bindings() {
        let source = "\
#!/bin/bash
x=\"${@:5}\"
y=\"prefix${@:2}suffix\"
declare z=\"${arr[@]:1}\"
readonly q=\"${arr[@]:1:2}\"
f() { local nested=\"${@:3}\"; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedArraySlice));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"${@:5}\"",
                "\"prefix${@:2}suffix\"",
                "\"${arr[@]:1}\"",
                "\"${arr[@]:1:2}\"",
                "\"${@:3}\"",
            ]
        );
    }

    #[test]
    fn ignores_replacement_star_and_non_scalar_contexts() {
        let source = "\
#!/bin/bash
x=\"${@:+fallback}\"
x=\"${arr[@]:+fallback}\"
x=\"${arr[*]:1}\"
x=\"\\$@\"
x=\"\\${@:2}\"
arr=(\"${@:2}\")
declare -a packed=(\"${arr[@]:1}\")
printf '%s\\n' \"${@:2}\"
if [ \"${arr[@]:1}\" = foo ]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedArraySlice));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_indexed_and_sliced_positional_parameter_assignments() {
        let source = "\
#!/bin/zsh
selected=\"${@[_i]}\"
selected_short=\"$@[_i]\"
scope=\"${@[_i]:-}\"
initial_query=\"${@[2,-1]:-}\"
fallback=\"${@[5,-1]:-fallback}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedArraySlice).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_zsh_positional_star_selector_assignments() {
        let source = "\
#!/bin/zsh
selected=\"${@[*]}\"
selected_short=\"$@[*]\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedArraySlice).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"${@[*]}\"", "\"$@[*]\""]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_intentional_join_selectors() {
        let source = "\
#!/bin/bash
x=\"$@\"
y=\"${@}\"
z=${@:5}
p=\"${arr[@]}\"
q=\"${arr[@]:-fallback}\"
flags+=\" ${add_flags[@]}\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::QuotedArraySlice),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 6);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
x=\"$*\"
y=\"${*}\"
z=${*:5}
p=\"${arr[*]}\"
q=\"${arr[*]:-fallback}\"
flags+=\" ${add_flags[*]}\"
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_zsh_star_selector_forms() {
        let source = "\
#!/bin/zsh
selected=\"${@[*]}\"
selected_short=\"$@[*]\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::QuotedArraySlice).with_shell(ShellDialect::Zsh),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/zsh
selected=\"${*[*]}\"
selected_short=\"$*[*]\"
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_replacement_and_array_assignment_forms_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
x=\"${@:+fallback}\"
arr=(\"${@:2}\")
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::QuotedArraySlice),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C099.sh").as_path(),
            &LinterSettings::for_rule(Rule::QuotedArraySlice),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C099_fix_C099.sh", result);
        Ok(())
    }
}
