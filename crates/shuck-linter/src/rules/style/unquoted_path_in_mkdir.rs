use shuck_ast::static_word_text;

use crate::{Checker, CommandFactRef, ExpansionContext, Rule, Violation, WordFactContext};

pub struct UnquotedPathInMkdir;

impl Violation for UnquotedPathInMkdir {
    fn rule() -> Rule {
        Rule::UnquotedPathInMkdir
    }

    fn message(&self) -> String {
        "quote mkdir path expansions".to_owned()
    }
}

pub fn unquoted_path_in_mkdir(checker: &mut Checker) {
    let source = checker.source();

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("mkdir"))
        .flat_map(|fact| mkdir_path_operand_spans(fact, source))
        .filter_map(|span| {
            checker.facts().word_fact(
                span,
                WordFactContext::Expansion(ExpansionContext::CommandArgument),
            )
        })
        .flat_map(|fact| fact.unquoted_scalar_expansion_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnquotedPathInMkdir);
}

fn mkdir_path_operand_spans(command: CommandFactRef<'_, '_>, source: &str) -> Vec<shuck_ast::Span> {
    let mut spans = Vec::new();
    let mut options_open = true;
    let mut expects_mode_operand = false;

    for word in command.body_args() {
        if expects_mode_operand {
            expects_mode_operand = false;
            continue;
        }

        let raw_text = word.span.slice(source);
        if options_open && (raw_text.starts_with("--mode=") || raw_text.starts_with("--context=")) {
            continue;
        }

        let Some(text) = static_word_text(word, source) else {
            spans.push(word.span);
            options_open = false;
            continue;
        };

        if options_open && text == "--" {
            options_open = false;
            continue;
        }

        if options_open && is_mkdir_option(word.span, text.as_ref(), &mut expects_mode_operand) {
            continue;
        }

        spans.push(word.span);
        options_open = false;
    }

    spans
}

fn is_mkdir_option(_span: shuck_ast::Span, text: &str, expects_mode_operand: &mut bool) -> bool {
    if !text.starts_with('-') || text == "-" {
        return false;
    }

    if text == "-m" || text == "--mode" {
        *expects_mode_operand = true;
        return true;
    }

    if text.starts_with("--mode=") {
        return true;
    }

    if let Some(short_cluster) = text.strip_prefix('-')
        && !short_cluster.starts_with('-')
    {
        if let Some(mode_index) = short_cluster.find('m')
            && short_cluster[mode_index + 1..].is_empty()
        {
            *expects_mode_operand = true;
        }
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_path_expansions_in_mkdir_operands() {
        let source = "\
#!/bin/sh
mkdir $dir
mkdir -p $PKG/var/lib/app
mkdir -m 750 prefix$leaf
mkdir --mode=700 ${root}/bin
command mkdir $other
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedPathInMkdir));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$dir", "$PKG", "$leaf", "${root}", "$other"]
        );
    }

    #[test]
    fn ignores_quoted_paths_and_mode_arguments() {
        let source = "\
#!/bin/sh
mkdir \"$dir\"
mkdir -- \"$dir\"
mkdir -m $mode \"$dir\"
mkdir --mode=$mode \"$dir\"
mkdir --mode \"$mode\" \"$dir\"
mkdir -pm 750 \"$dir\"
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedPathInMkdir));

        let slices = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.span.slice(source))
            .collect::<Vec<_>>();
        assert_eq!(slices, Vec::<&str>::new());
    }
}
