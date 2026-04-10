use crate::{Checker, Rule, Violation, static_word_text};

pub struct SuWithoutFlag;

impl Violation for SuWithoutFlag {
    fn rule() -> Rule {
        Rule::SuWithoutFlag
    }

    fn message(&self) -> String {
        "use `su -l` or `su -c` when switching users".to_owned()
    }
}

pub fn su_without_flag(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("su") && fact.wrappers().is_empty())
        .filter(|fact| !su_has_login_or_command_flag(fact, checker.source()))
        .filter_map(|fact| fact.body_name_word().map(|word| word.span))
        .collect::<Vec<_>>();

    checker.report_all(spans, || SuWithoutFlag);
}

fn su_has_login_or_command_flag(fact: &crate::CommandFact<'_>, source: &str) -> bool {
    let args = fact.body_args();
    args.iter().enumerate().any(|(index, word)| {
        let Some(text) = static_word_text(word, source) else {
            return false;
        };

        is_login_or_command_flag(text.as_str()) && args[index + 1..].iter().any(|_| true)
    })
}

fn is_login_or_command_flag(text: &str) -> bool {
    matches!(text, "-" | "-l" | "--login" | "-c" | "--command")
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_plain_su_invocations_without_login_or_command_flags() {
        let source = "\
#!/bin/bash
su librenms
su -
su -c
su -l
command su librenms
sudo su librenms
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SuWithoutFlag));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["su", "su", "su", "su"]
        );
    }

    #[test]
    fn ignores_login_and_command_forms() {
        let source = "\
#!/bin/bash
su -l root
su --login root
su -c id root
su --command id root
su - root
su librenms -c id
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SuWithoutFlag));

        assert!(diagnostics.is_empty());
    }
}
