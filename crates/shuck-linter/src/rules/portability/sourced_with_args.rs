use shuck_ast::static_word_text;

use crate::context::FileContextTag;
use crate::{Checker, Rule, ShellDialect, Violation};

pub struct SourcedWithArgs;

impl Violation for SourcedWithArgs {
    fn rule() -> Rule {
        Rule::SourcedWithArgs
    }

    fn message(&self) -> String {
        "sourced files do not accept extra arguments in POSIX sh".to_owned()
    }
}

pub fn sourced_with_args(checker: &mut Checker) {
    if !targets_posix_dot_shell(checker.shell()) {
        return;
    }
    if checker.file_context().has_tag(FileContextTag::PatchFile) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| {
            fact.body_name_word()
                .and_then(|word| static_command_name_text(word, checker.source()))
                .as_deref()
                == Some(".")
        })
        .filter_map(|fact| fact.body_args().get(1).copied())
        .map(|word| word.span)
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SourcedWithArgs);
}

fn targets_posix_dot_shell(shell: ShellDialect) -> bool {
    matches!(shell, ShellDialect::Sh | ShellDialect::Dash)
}

fn static_command_name_text(word: &shuck_ast::Word, source: &str) -> Option<String> {
    let text = static_word_text(word, source)?;
    Some(text.strip_prefix('\\').unwrap_or(text.as_ref()).to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::LinterSettings;
    use crate::test::{test_snippet, test_snippet_at_path};

    #[test]
    fn ignores_extra_arguments_in_bash() {
        let source = "#!/bin/bash\n. ./helper.sh foo\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SourcedWithArgs).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_escaped_dot_inside_command_substitution() {
        let source = "#!/bin/sh\n[ \"_$(echo 'echo $1' | \\. /dev/stdin yes)\" = \"_yes\" ]\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SourcedWithArgs));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "yes");
    }

    #[test]
    fn ignores_patch_file_context() {
        let source = "#! /bin/sh /usr/share/dpatch/dpatch-run\nat configure time by the --with-conf=<file> argument but defaults to\n";
        let diagnostics = test_snippet_at_path(
            Path::new("example.patch"),
            source,
            &LinterSettings::for_rule(Rule::SourcedWithArgs),
        );

        assert!(diagnostics.is_empty());
    }
}
