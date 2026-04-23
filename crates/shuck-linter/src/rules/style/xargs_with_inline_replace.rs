use crate::{Checker, Rule, ShellDialect, Violation};

pub struct XargsWithInlineReplace;

impl Violation for XargsWithInlineReplace {
    fn rule() -> Rule {
        Rule::XargsWithInlineReplace
    }

    fn message(&self) -> String {
        "replace deprecated `xargs -i` with `xargs -I{}`".to_owned()
    }
}

pub fn xargs_with_inline_replace(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    ) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.options().xargs())
        .flat_map(|xargs| xargs.inline_replace_option_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || XargsWithInlineReplace);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_inline_replace_xargs_flags() {
        let source = "\
#!/bin/sh
find . -type d -name CVS | xargs -iX rm -rf X
find . -type d -name CVS | xargs -0iX rm -rf X
find . -type d -name CVS | xargs -i{} rm -rf '{}'
command xargs -i echo {}
sudo xargs -i echo {}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::XargsWithInlineReplace),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-iX", "-0iX", "-i{}", "-i", "-i"]
        );
    }

    #[test]
    fn follows_shellcheck_silent_wrapper_cases() {
        let source = "\
#!/bin/sh
find . -type f | xargs -i bash -c 'echo {}'
find . -type f | xargs -0i sh -c 'echo {}'
xargs -i echo '-----> Configuring {}'
xargs -0i echo '-----> Configuring {}'
xargs -i echo \"-----> Configuring {} with $template\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::XargsWithInlineReplace),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_modern_xargs_replace_flags() {
        let source = "\
#!/bin/sh
find . -type d -name CVS | xargs -I{} rm -rf {}
find . -type d -name CVS | xargs --replace rm -rf {}
find . -type d -name CVS | xargs --replace={} rm -rf '{}'
find . -type d -name CVS | xargs -0 rm -rf
find . -type d -name CVS | xargs --null rm -rf
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::XargsWithInlineReplace),
        );

        assert!(diagnostics.is_empty());
    }
}
