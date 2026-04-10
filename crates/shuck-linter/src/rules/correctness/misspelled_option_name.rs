use crate::{Checker, Rule, Violation};

pub struct MisspelledOptionName;

impl Violation for MisspelledOptionName {
    fn rule() -> Rule {
        Rule::MisspelledOptionName
    }

    fn message(&self) -> String {
        "configure option name looks misspelled".to_owned()
    }
}

pub fn misspelled_option_name(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.options().configure())
        .flat_map(|configure| configure.misspelled_option_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || MisspelledOptionName);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_misspelled_configure_option_names() {
        let source = "\
./configure --with-optmizer=${CFLAGS}
configure \"--enable-optmizer=${CFLAGS}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MisspelledOptionName),
        );

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["--with-optmizer", "--enable-optmizer"]
        );
    }

    #[test]
    fn ignores_non_configure_commands_or_known_option_spellings() {
        let source = "\
./configure --with-optimizer=${CFLAGS}
configure --disable-optimizer
make --with-optmizer=${CFLAGS}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MisspelledOptionName),
        );

        assert!(diagnostics.is_empty());
    }
}
