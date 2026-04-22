use crate::{Checker, Rule, ShellDialect, Violation};

pub struct StarGlobRemovalInSh;

impl Violation for StarGlobRemovalInSh {
    fn rule() -> Rule {
        Rule::StarGlobRemovalInSh
    }

    fn message(&self) -> String {
        "prefix/suffix trimming on `$@` or `$*` is not portable in `sh`".to_owned()
    }
}

pub fn star_glob_removal_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .star_glob_removal_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || StarGlobRemovalInSh);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_positional_parameter_trim_operations() {
        let source = "\
#!/bin/sh
printf '%s\n' \"${*%%dBm*}\" \"${*%dBm*}\" \"${*##dBm*}\" \"${*#dBm*}\" \"${@%%dBm*}\" \"${@%dBm*}\" \"${@##*.}\" \"${@#*.}\" \"${name%%dBm*}\" \"${*//dBm/x}\"
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::StarGlobRemovalInSh));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${*%%dBm*}",
                "${*%dBm*}",
                "${*##dBm*}",
                "${*#dBm*}",
                "${@%%dBm*}",
                "${@%dBm*}",
                "${@##*.}",
                "${@#*.}",
            ]
        );
    }

    #[test]
    fn ignores_star_glob_removal_in_bash() {
        let source = "printf '%s\\n' \"${*%%dBm*}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StarGlobRemovalInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
