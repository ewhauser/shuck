use rustc_hash::FxHashSet;

use crate::{
    Checker, CommandFact, ExpansionContext, Rule, ShellDialect, Violation, WordQuote,
    static_word_text,
};

pub struct UnquotedVariableInSed;

impl Violation for UnquotedVariableInSed {
    fn rule() -> Rule {
        Rule::UnquotedVariableInSed
    }

    fn message(&self) -> String {
        "quote unquoted variables before piping them to sed".to_owned()
    }
}

pub fn unquoted_variable_in_sed(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Bash | ShellDialect::Ksh) {
        return;
    }

    let source = checker.source();
    let qualifying_echo_commands = checker
        .facts()
        .pipelines()
        .iter()
        .flat_map(|pipeline| {
            pipeline.segments().windows(2).filter_map(|pair| {
                let left = checker.facts().command(pair[0].command_id());
                let right = checker.facts().command(pair[1].command_id());

                if !is_plain_command_named(left, "echo")
                    || !is_plain_command_named(right, "sed")
                    || !sed_uses_static_substitution_script(right, source)
                {
                    return None;
                }

                Some(left.id())
            })
        })
        .collect::<FxHashSet<_>>();

    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .filter(|fact| {
            qualifying_echo_commands.contains(&fact.command_id())
                && fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                && fact.classification().quote == WordQuote::Unquoted
                && fact.classification().has_scalar_expansion()
        })
        .flat_map(|fact| fact.scalar_expansion_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnquotedVariableInSed);
}

fn is_plain_command_named(fact: &CommandFact<'_>, name: &str) -> bool {
    fact.effective_name_is(name) && fact.wrappers().is_empty()
}

fn sed_uses_static_substitution_script(fact: &CommandFact<'_>, source: &str) -> bool {
    let mut expects_script = true;

    for word in fact.body_args() {
        let Some(text) = static_word_text(word, source) else {
            continue;
        };

        if text == "-e" {
            expects_script = true;
            continue;
        }

        if let Some(script) = attached_sed_script(text.as_str()) {
            if is_static_substitution_script(script) {
                return true;
            }
            expects_script = false;
            continue;
        }

        if expects_script && is_static_substitution_script(&text) {
            return true;
        }

        if !text.starts_with('-') {
            expects_script = false;
        }
    }

    false
}

fn attached_sed_script(text: &str) -> Option<&str> {
    let flags = text.strip_prefix('-')?;

    for (index, flag) in flags.char_indices() {
        if flag == 'e' {
            let script = &flags[index + flag.len_utf8()..];
            return (!script.is_empty()).then_some(script);
        }
    }

    None
}

fn is_static_substitution_script(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 4 || bytes[0] != b's' {
        return false;
    }

    let delimiter = bytes[1];
    if delimiter.is_ascii_whitespace() || delimiter == b'\\' {
        return false;
    }

    let mut sections = 0;
    let mut index = 2;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                index = index.saturating_add(2);
                continue;
            }
            byte if byte == delimiter => {
                sections += 1;
                if sections == 2 {
                    return true;
                }
            }
            _ => {}
        }
        index += 1;
    }

    false
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_unquoted_variable_expansions_before_sed_substitution() {
        let source = "\
#!/bin/bash
echo $CLASSPATH | sed 's|foo|bar|g'
echo $HOME | sed -e 's|foo|bar|g'
echo $USER | sed -e's/foo/bar/'
echo $SHELL | sed -es/foo/bar/
echo \"$KEEP\" | sed 's|foo|bar|g'
echo $PATH | sed -n
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedVariableInSed),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$CLASSPATH", "$HOME", "$USER", "$SHELL"]
        );
    }

    #[test]
    fn ignores_non_qualifying_shells_and_non_substitution_sed() {
        let source = "\
#!/bin/sh
echo $CLASSPATH | sed 's|foo|bar|g'
echo $CLASSPATH | sed -n
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedVariableInSed).with_shell(ShellDialect::Sh),
        );

        assert!(diagnostics.is_empty());
    }
}
