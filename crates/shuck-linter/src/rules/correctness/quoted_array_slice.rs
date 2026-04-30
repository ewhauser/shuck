use crate::{Checker, ExpansionContext, Rule, Violation, WordFactHostKind};

pub struct QuotedArraySlice;

impl Violation for QuotedArraySlice {
    fn rule() -> Rule {
        Rule::QuotedArraySlice
    }

    fn message(&self) -> String {
        "all-elements array expansions collapse in scalar assignment values".to_owned()
    }
}

pub fn quoted_array_slice(checker: &mut Checker) {
    let facts = checker.facts();
    let locator = checker.locator();
    let spans = [
        ExpansionContext::AssignmentValue,
        ExpansionContext::DeclarationAssignmentValue,
    ]
    .into_iter()
    .flat_map(|context| facts.expansion_word_facts(context))
    .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
    .filter(|fact| !facts.is_compound_assignment_value_word(*fact))
    .filter(|fact| fact.has_direct_all_elements_array_expansion_in_source(locator))
    .map(|fact| fact.span())
    .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || QuotedArraySlice);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
}
