use rustc_hash::FxHashMap;
use shuck_ast::{RedirectKind, Span};

use crate::rules::common::expansion::{ComparablePathKey, ExpansionContext, comparable_path};
use crate::{Checker, Rule, Violation};

pub struct RedirectClobbersInput;

impl Violation for RedirectClobbersInput {
    fn rule() -> Rule {
        Rule::RedirectClobbersInput
    }

    fn message(&self) -> String {
        "this command reads and writes the same file".to_owned()
    }
}

pub fn redirect_clobbers_input(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .structural_commands()
        .flat_map(|fact| clobber_spans_for_command(fact, source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || RedirectClobbersInput);
}

fn clobber_spans_for_command(fact: &crate::CommandFact<'_>, source: &str) -> Vec<Span> {
    if fact.effective_name_is("echo") || fact.effective_name_is("printf") {
        return Vec::new();
    }

    let mut read_paths: FxHashMap<ComparablePathKey, Vec<Span>> = FxHashMap::default();
    let mut write_paths: FxHashMap<ComparablePathKey, Vec<Span>> = FxHashMap::default();
    let mut readwrite_paths: FxHashMap<ComparablePathKey, Vec<Span>> = FxHashMap::default();

    let options = fact.zsh_options();
    for redirect in fact.redirect_facts() {
        let Some(target) = redirect.redirect().word_target() else {
            continue;
        };
        let Some(comparable) = comparable_path(
            target,
            source,
            ExpansionContext::from_redirect_kind(redirect.redirect().kind)
                .expect("redirect kinds with word targets should have a context"),
            options,
        ) else {
            continue;
        };

        let key = comparable.key().clone();
        match redirect.redirect().kind {
            RedirectKind::Input => {
                read_paths.entry(key).or_default().push(comparable.span());
            }
            RedirectKind::ReadWrite => {
                readwrite_paths
                    .entry(key)
                    .or_default()
                    .push(comparable.span());
            }
            RedirectKind::Output | RedirectKind::Clobber | RedirectKind::Append => {
                write_paths.entry(key).or_default().push(comparable.span());
            }
            RedirectKind::OutputBoth => {
                write_paths.entry(key).or_default().push(comparable.span());
            }
            RedirectKind::HereDoc
            | RedirectKind::HereDocStrip
            | RedirectKind::HereString
            | RedirectKind::DupOutput
            | RedirectKind::DupInput => {}
        }
    }

    for source_word in fact.scope_read_source_words() {
        let Some(comparable) =
            comparable_path(source_word.word(), source, source_word.context(), options)
        else {
            continue;
        };

        read_paths
            .entry(comparable.key().clone())
            .or_default()
            .push(comparable.span());
    }

    let mut spans = Vec::new();
    for (key, read_spans) in &read_paths {
        let Some(write_spans) = write_paths.get(key) else {
            continue;
        };

        spans.extend(read_spans.iter().copied());
        spans.extend(write_spans.iter().copied());
    }

    for (key, readwrite_spans) in readwrite_paths {
        if read_paths.contains_key(&key) || write_paths.contains_key(&key) {
            spans.extend(readwrite_spans);
        }
    }

    spans
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_input_and_output_paths_that_match() {
        let source = "\
#!/bin/bash
cat < foo > foo
sort foo > foo
unzip -p \"$1\" test.c > test.c
cat < \"$src\" > \"$src\"
sed -e 's/x/y/' foo > foo
awk -f prog.awk data.txt > data.txt
echo \"$(cat \"$f\")\" | sed 's/x/y/' >\"$f\"
printf '%s\\0' **/* | bsdtar --null --files-from - --exclude .MTREE | gzip -c -f -n > .MTREE
{ [[ \"$f\" == /dev/null ]] || set -x; } &>\"$f\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedirectClobbersInput),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "foo", "foo", "foo", "foo", "test.c", "test.c", "\"$src\"", "\"$src\"", "foo",
                "foo", "data.txt", "data.txt", "\"$f\"", "\"$f\"", ".MTREE", ".MTREE", "\"$f\"",
                "\"$f\"",
            ]
        );
    }

    #[test]
    fn ignores_commands_without_matching_read_and_write_paths() {
        let source = "\
#!/bin/bash
exec 4<> \"$LOG_PATH\"
cat < foo > bar
sort foo > bar
echo foo > foo
printf '%s\\n' foo > foo
cat < \"$src\" > \"$dst\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedirectClobbersInput),
        );

        assert!(diagnostics.is_empty());
    }
}
