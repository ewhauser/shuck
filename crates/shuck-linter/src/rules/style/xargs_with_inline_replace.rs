use crate::{Checker, Rule, ShellDialect, Violation, facts::leading_literal_word_prefix};
use shuck_ast::{Word, static_word_text};

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

    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.options().xargs())
        .flat_map(|xargs| {
            xargs
                .inline_replace_options()
                .iter()
                .copied()
                .filter(move |option| {
                    !option.uses_default_replacement()
                        || !matches_sc2267_default_replace_silent_shape(
                            xargs.command_operand_words(),
                            source,
                        )
                })
                .map(|option| option.span())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || XargsWithInlineReplace);
}

fn matches_sc2267_default_replace_silent_shape(args: &[&Word], source: &str) -> bool {
    matches_shell_c_wrapper(args, source) || matches_echo_leading_dash_replacement(args, source)
}

fn matches_shell_c_wrapper(args: &[&Word], source: &str) -> bool {
    let args = if args
        .first()
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        == Some("command")
    {
        &args[1..]
    } else {
        args
    };

    let Some(command_name) = args.first().and_then(|word| static_word_text(word, source)) else {
        return false;
    };

    matches!(
        command_basename(command_name.as_ref()),
        "sh" | "bash" | "dash" | "ksh" | "zsh"
    ) && args
        .get(1)
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        == Some("-c")
}

fn matches_echo_leading_dash_replacement(args: &[&Word], source: &str) -> bool {
    let Some(command_name) = args.first().and_then(|word| static_word_text(word, source)) else {
        return false;
    };

    if command_basename(command_name.as_ref()) != "echo" {
        return false;
    }

    let Some(first_operand) = args.get(1) else {
        return false;
    };
    let literal_prefix = leading_literal_word_prefix(first_operand, source);
    literal_prefix.starts_with('-') && literal_prefix != "-" && literal_prefix.contains("{}")
}

fn command_basename(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
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
find . -type f | xargs -i /bin/sh -c 'echo {}'
find . -type f | xargs -i command sh -c 'echo {}'
xargs -i echo '-----> Configuring {}'
xargs -0i echo '-----> Configuring {}'
xargs -i echo \"-----> Configuring {} with $template\"
xargs -i /bin/echo '-----> Configuring {}'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::XargsWithInlineReplace),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_default_replace_outside_silent_sc2267_shapes() {
        let source = "\
#!/bin/sh
xargs -i env sh -c 'echo {}'
xargs -i sudo sh -c 'echo {}'
xargs -i sh -ec 'echo {}'
xargs -i echo '{}'
xargs -i echo -n '-x {}'
xargs -i echo -- '-x {}'
xargs -i command echo '-----> Configuring {}'
xargs -i printf -- '-x %s\n' '{}'
xargs -i{} sh -c 'echo {}'
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
            vec!["-i", "-i", "-i", "-i", "-i", "-i", "-i", "-i", "-i{}"]
        );
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
