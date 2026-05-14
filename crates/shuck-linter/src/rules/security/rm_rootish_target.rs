use crate::{Checker, Rule, Violation};

pub struct RmRootishTarget;

impl Violation for RmRootishTarget {
    fn rule() -> Rule {
        Rule::RmRootishTarget
    }

    fn message(&self) -> String {
        "recursive `rm` targets a root-like directory".to_owned()
    }
}

pub fn rm_rootish_target(checker: &mut Checker) {
    let spans = checker
        .facts()
        .command_facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("rm"))
        .filter_map(|fact| fact.options().rm())
        .flat_map(|rm| rm.rootish_path_spans(checker.source()).iter().copied())
        .collect::<Vec<_>>();

    checker.report_all(spans, || RmRootishTarget);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_recursive_rm_on_root_or_home_roots() {
        let source = "#!/bin/sh\nrm -rf /\nrm -rf /*\nrm -rf /**\nrm -rf /./\nrm -rf /./*\nrm -rf /..\nrm -rf /../*\nrm -rf \"$HOME\"\nrm -rf \"${HOME}\"/*\nrm -rf \"${HOME}\"/**\nrm -rf \"$HOME\"/.\nrm -rf \"$HOME\"/./*\nrm -rf \"${HOME:?}\"\nrm -rf \"${HOME:?}\"/*\nrm -rf \"${HOME:?}\"/.\nrm -rf ~\nrm -rf ~/*\nrm -rf ~/**\nrm -rf ~/.\nrm -rf ~/./*\nrm -rf ~root/*\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::RmRootishTarget));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "/",
                "/*",
                "/**",
                "/./",
                "/./*",
                "/..",
                "/../*",
                "\"$HOME\"",
                "\"${HOME}\"/*",
                "\"${HOME}\"/**",
                "\"$HOME\"/.",
                "\"$HOME\"/./*",
                "\"${HOME:?}\"",
                "\"${HOME:?}\"/*",
                "\"${HOME:?}\"/.",
                "~",
                "~/*",
                "~/**",
                "~/.",
                "~/./*",
                "~root/*"
            ]
        );
    }

    #[test]
    fn ignores_non_recursive_and_bounded_rm_targets() {
        let source = "#!/bin/sh\ndir=/tmp\nrm -f \"$HOME\"/*\nrm -rf \"$HOME\"/.cache\nrm -rf \"${HOME%/*}\"\nrm -rf \"${HOME%%/*}\"\nrm -rf \"$HOME\"/..\nrm -rf \"$HOME\"/../*\nrm -rf ~/..\nrm -rf ~/../*\nrm -rf ~0\nrm -rf ~1/*\nrm -rf ~/Downloads/*\nrm -rf /tmp/*\nrm -rf /tmp/../*\nrm -rf ~/tmp/../*\nrm -rf ~root/tmp/../*\nrm -rf \"$dir\"/*\nrm -rf \"~\"/*\nrm -rf \"$HOME\"/\"*\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::RmRootishTarget));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
