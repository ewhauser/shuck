use crate::{Checker, Rule, Violation};

pub struct GlobInFindSubstitution;

impl Violation for GlobInFindSubstitution {
    fn rule() -> Rule {
        Rule::GlobInFindSubstitution
    }

    fn message(&self) -> String {
        "quote glob patterns passed to `find` so the shell does not expand them early".to_owned()
    }
}

pub fn glob_in_find_substitution(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("find") && fact.wrappers().is_empty())
        .filter_map(|fact| fact.options().find())
        .flat_map(|find| find.glob_pattern_operand_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GlobInFindSubstitution);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_find_pattern_operands_that_can_glob_expand() {
        let source = "\
#!/bin/bash
find ./ -name *.jar
find ./ -name \"$prefix\"*.jar
find ./ -wholename */tmp/*
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
            vec!["*.jar", "\"$prefix\"*.jar", "*/tmp/*", "*.cfg", "*/tmp/*"]
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
}
