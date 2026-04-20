use crate::{Checker, Rule, ShellDialect, Violation};

pub struct ArrayReference;

impl Violation for ArrayReference {
    fn rule() -> Rule {
        Rule::ArrayReference
    }

    fn message(&self) -> String {
        "array references are not portable in `sh`".to_owned()
    }
}

pub fn array_reference(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .indexed_array_reference_fragments()
        .iter()
        .filter(|fragment| fragment.is_plain())
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ArrayReference);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_plain_array_references_only() {
        let source = "\
#!/bin/sh
printf '%s\n' \"${arr[0]}\" \"${arr[@]}\" \"${arr[*]}\" \"${#arr[0]}\" \"${#arr[@]}\" \"${arr[0]%x}\" \"${arr[0]:2}\" \"${arr[0]//x/y}\" \"${arr[0]:-fallback}\" \"${!arr[0]}\" \"${!arr[@]}\"\n\
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ArrayReference));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[0]}", "${arr[@]}", "${arr[*]}"]
        );
    }

    #[test]
    fn ignores_indexed_array_references_in_bash() {
        let source = "printf '%s\n' \"${arr[0]}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayReference).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
