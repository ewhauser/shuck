use crate::{
    Checker, CommandFact, ExpansionContext, Rule, ShellDialect, Violation, WordFactContext,
    static_word_text,
};

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

    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|command| ln_symlink_target_words(command, source))
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

fn ln_symlink_target_words<'a>(
    command: &'a CommandFact<'a>,
    source: &str,
) -> Vec<&'a shuck_ast::Word> {
    if !command.effective_name_is("ln") {
        return Vec::new();
    }

    let args = command.body_args();
    let mut index = 0usize;
    let mut saw_symbolic_flag = false;
    let mut target_directory_mode = false;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        if let Some(long) = text.strip_prefix("--") {
            match long {
                "symbolic" => saw_symbolic_flag = true,
                "target-directory" => {
                    target_directory_mode = true;
                    index += 1;
                    if args.get(index).is_none() {
                        return Vec::new();
                    }
                }
                "suffix" => {
                    index += 1;
                    if args.get(index).is_none() {
                        return Vec::new();
                    }
                }
                "backup"
                | "directory"
                | "force"
                | "interactive"
                | "logical"
                | "no-dereference"
                | "no-target-directory"
                | "physical"
                | "relative"
                | "verbose" => {}
                _ if long.starts_with("target-directory=") => {
                    target_directory_mode = true;
                }
                _ if long.starts_with("suffix=") => {}
                _ => return Vec::new(),
            }

            index += 1;
            continue;
        }

        let mut chars = text[1..].chars().peekable();
        while let Some(flag) = chars.next() {
            match flag {
                's' => saw_symbolic_flag = true,
                't' => {
                    target_directory_mode = true;
                    if chars.peek().is_none() {
                        index += 1;
                        if args.get(index).is_none() {
                            return Vec::new();
                        }
                    }
                    break;
                }
                'S' => {
                    if chars.peek().is_none() {
                        index += 1;
                        if args.get(index).is_none() {
                            return Vec::new();
                        }
                    }
                    break;
                }
                'b' | 'd' | 'f' | 'F' | 'i' | 'L' | 'n' | 'P' | 'r' | 'T' | 'v' => {}
                _ => return Vec::new(),
            }
        }

        index += 1;
    }

    if !saw_symbolic_flag {
        return Vec::new();
    }

    let operands = &args[index..];
    if operands.is_empty() {
        return Vec::new();
    }

    if target_directory_mode {
        operands.to_vec()
    } else {
        operands.first().copied().into_iter().collect()
    }
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
