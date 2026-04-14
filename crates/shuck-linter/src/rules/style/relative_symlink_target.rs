use crate::{Checker, ExpansionContext, Rule, ShellDialect, Violation, WordFactContext};

pub struct RelativeSymlinkTarget;

impl Violation for RelativeSymlinkTarget {
    fn rule() -> Rule {
        Rule::RelativeSymlinkTarget
    }

    fn message(&self) -> String {
        "avoid deep relative paths in symlink targets".to_owned()
    }
}

pub fn relative_symlink_target(checker: &mut Checker) {
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
        .filter_map(|command| command.options().ln())
        .flat_map(|ln| ln.symlink_target_words().iter().copied())
        .filter_map(|word| {
            let fact = checker.facts().word_fact(
                word.span,
                WordFactContext::Expansion(ExpansionContext::CommandArgument),
            )?;
            if !fact.classification().is_fixed_literal()
                || fact.runtime_literal().is_runtime_sensitive()
            {
                return None;
            }

            let text = fact.static_text()?;
            is_deep_relative_target(text).then_some(word.span)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || RelativeSymlinkTarget);
}

fn is_deep_relative_target(text: &str) -> bool {
    let mut offset = 0usize;
    let mut parents = 0usize;

    while let Some(remaining) = text.get(offset..) {
        if !remaining.starts_with("..") {
            break;
        }
        let suffix = &remaining[2..];
        if !suffix.is_empty() && !suffix.starts_with('/') {
            break;
        }

        parents += 1;
        offset += 2;
        if text.get(offset..).is_some_and(|tail| tail.starts_with('/')) {
            offset += 1;
        }
    }

    parents >= 2
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_deep_relative_symlink_targets() {
        let source = "\
#!/bin/sh
ln -s ../../share/doc/guide.pdf
ln -s ../.. parent-only
ln -s ../../.. parent-only-3
ln -snf ../../etc/defaults cfg-link
ln --symbolic ../../lib/pkg app
ln -st /tmp ../../alpha ../../beta
ln -s -- ../../share/doc/guide.pdf guide
command ln -s ../../wrapped/value wrapped
sudo ln -s ../../wrapped/value wrapped
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RelativeSymlinkTarget),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "../../share/doc/guide.pdf",
                "../..",
                "../../..",
                "../../etc/defaults",
                "../../lib/pkg",
                "../../alpha",
                "../../beta",
                "../../share/doc/guide.pdf",
                "../../wrapped/value",
                "../../wrapped/value",
            ]
        );
    }

    #[test]
    fn ignores_non_deep_or_dynamic_or_non_symbolic_targets() {
        let source = "\
#!/bin/sh
ln -s ../share/doc/guide.pdf guide
ln -s ./share/doc/guide.pdf guide
ln -s /usr/share/doc/guide.pdf guide
ln ../../share/doc/guide.pdf guide
ln -s \"$base\"/../../guide.pdf guide
ln -s .. parent-only
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RelativeSymlinkTarget),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn does_not_run_for_zsh() {
        let source = "\
#!/bin/zsh
ln -s ../../share/doc/guide.pdf guide
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RelativeSymlinkTarget).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }
}
