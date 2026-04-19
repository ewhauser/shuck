use crate::{Checker, Rule, Violation};

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
        .filter(|fact| {
            fact.options()
                .su()
                .is_some_and(|su| !su.has_login_or_command_flag())
        })
        .map(|fact| fact.span_in_source(checker.source()))
        .collect::<Vec<_>>();

    checker.report_all(spans, || SuWithoutFlag);
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
su -c
su --command
su -- root echo -c hi
command su librenms
sudo su librenms
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SuWithoutFlag));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "su librenms",
                "su -c",
                "su --command",
                "su -- root echo -c hi"
            ]
        );
    }

    #[test]
    fn ignores_login_and_command_forms() {
        let source = "\
#!/bin/bash
su -
su -l
su -l root
su --login root
su -c id root
su -cid root
su --command id root
su - root
su root -c id
su alice -
su \"$user\" -c \"$cmd\"
su \"$user\" -s /bin/sh -c \"$cmd\"
bundle_dir=$(su \"$user\" -s \"$SHELL\" -c \"echo ~/.bundle\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SuWithoutFlag));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn still_reports_forms_without_login_or_command_flags() {
        let source = "\
#!/bin/bash
su -m root
su -s /bin/sh root
su alice -- -
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SuWithoutFlag));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["su -m root", "su -s /bin/sh root", "su alice -- -"]
        );
    }
}
