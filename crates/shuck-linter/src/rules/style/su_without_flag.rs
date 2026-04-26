use crate::{Checker, Rule, Violation};
use rustc_hash::FxHashSet;

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
    let piped_command_ids = checker
        .facts()
        .pipelines()
        .iter()
        .flat_map(|pipeline| {
            pipeline
                .segments()
                .iter()
                .map(|segment| segment.command_id())
        })
        .collect::<FxHashSet<_>>();

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("su") && fact.wrappers().is_empty())
        .filter(|fact| fact.options().su().is_some_and(|su| !su.has_login_flag()))
        .filter(|fact| !piped_command_ids.contains(&fact.id()))
        .filter(|fact| !fact.has_redirects())
        .filter(|fact| fact.body_args().len() < 2)
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
su
su librenms
su -m
su --pty
su -c
su --command
su -s
su --version
su --help
su --
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
                "su",
                "su librenms",
                "su -m",
                "su --pty",
                "su -c",
                "su --command",
                "su -s",
                "su --version",
                "su --help",
                "su --"
            ]
        );
    }

    #[test]
    fn ignores_login_and_explicit_multi_word_forms() {
        let source = "\
#!/bin/bash
su -
su -l
su -l root
su --login root
su -c id root
su -cid root
su --command id root
su -m root
su --pty root
su -s /bin/sh
su - root
su -- root
su root -m
su root -c id
su root -c
su root -s /bin/sh
su root bash -c 'id'
su -- root echo -c hi
su alice -
su \"$user\" -c \"$cmd\"
su \"$user\" -s /bin/sh -c \"$cmd\"
bundle_dir=$(su \"$user\" -s \"$SHELL\" -c \"echo ~/.bundle\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SuWithoutFlag));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_pipelined_and_redirected_su_invocations() {
        let source = "\
#!/bin/bash
su root >/dev/null 2>&1
su --version >/dev/null 2>&1
su root | cat
cat file | su root
! su --version | grep -q util-linux
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SuWithoutFlag));

        assert!(diagnostics.is_empty());
    }
}
