use rustc_hash::FxHashMap;
use shuck_ast::{RedirectKind, Span};

use crate::{Checker, ComparablePathKey, ExpansionContext, PathNameKind, Rule, Violation};

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
    let spans = checker
        .facts()
        .structural_commands()
        .flat_map(clobber_spans_for_command)
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || RedirectClobbersInput);
}

fn clobber_spans_for_command(fact: &crate::CommandFact<'_>) -> Vec<Span> {
    if fact.effective_name_is("echo") || fact.effective_name_is("printf") {
        return Vec::new();
    }

    let mut read_paths: FxHashMap<ComparablePathKey, Vec<Span>> = FxHashMap::default();
    let mut write_paths: FxHashMap<ComparablePathKey, Vec<Span>> = FxHashMap::default();
    let mut readwrite_paths: FxHashMap<ComparablePathKey, Vec<Span>> = FxHashMap::default();
    let mut read_names: FxHashMap<Box<str>, Vec<(PathNameKind, Span)>> = FxHashMap::default();
    let mut write_names: FxHashMap<Box<str>, Vec<(PathNameKind, Span)>> = FxHashMap::default();
    let own_readwrite_spans = fact
        .redirect_facts()
        .iter()
        .filter(|redirect| redirect.redirect().kind == RedirectKind::ReadWrite)
        .filter_map(|redirect| redirect.redirect().word_target().map(|word| word.span))
        .collect::<Vec<_>>();

    for redirect in fact.redirect_facts() {
        let Some(comparable) = redirect.comparable_path() else {
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
        if source_word.context() == ExpansionContext::RedirectTarget(RedirectKind::ReadWrite)
            && own_readwrite_spans.contains(&source_word.word().span)
        {
            continue;
        }

        let Some(comparable) = source_word.comparable_path() else {
            continue;
        };

        read_paths
            .entry(comparable.key().clone())
            .or_default()
            .push(comparable.span());
    }

    for source_name in fact.scope_read_source_names() {
        read_names
            .entry(source_name.name().into())
            .or_default()
            .push((source_name.kind(), source_name.span()));
    }

    for target_name in fact.scope_write_target_names() {
        write_names
            .entry(target_name.name().into())
            .or_default()
            .push((target_name.kind(), target_name.span()));
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

    for (name, read_spans) in &read_names {
        let Some(write_spans) = write_names.get(name) else {
            continue;
        };
        if !name_spans_overlap_shellcheck_style(read_spans, write_spans) {
            continue;
        }

        spans.extend(read_spans.iter().map(|(_, span)| *span));
        spans.extend(write_spans.iter().map(|(_, span)| *span));
    }

    spans
}

fn name_spans_overlap_shellcheck_style(
    read_spans: &[(PathNameKind, Span)],
    write_spans: &[(PathNameKind, Span)],
) -> bool {
    read_spans.iter().any(|(read_kind, read_span)| {
        write_spans.iter().any(|(write_kind, write_span)| {
            read_span != write_span && path_name_kinds_match(*read_kind, *write_kind)
        })
    })
}

fn path_name_kinds_match(read_kind: PathNameKind, write_kind: PathNameKind) -> bool {
    match write_kind {
        PathNameKind::Literal => matches!(
            read_kind,
            PathNameKind::Literal
                | PathNameKind::Parameter
                | PathNameKind::RedirectLiteral
                | PathNameKind::RedirectParameter
                | PathNameKind::HeredocParameter
        ),
        PathNameKind::Parameter => {
            matches!(
                read_kind,
                PathNameKind::Parameter | PathNameKind::RedirectParameter
            )
        }
        PathNameKind::RedirectLiteral
        | PathNameKind::QuotedRedirectLiteral
        | PathNameKind::RedirectParameter
        | PathNameKind::HeredocParameter => false,
        PathNameKind::GeneratedLiteral => matches!(read_kind, PathNameKind::RedirectLiteral),
        PathNameKind::GeneratedParameter => {
            matches!(
                read_kind,
                PathNameKind::RedirectLiteral | PathNameKind::RedirectParameter
            )
        }
        PathNameKind::BindingTarget => matches!(read_kind, PathNameKind::RedirectLiteral),
    }
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
sed -es/x/y/ foo > foo
awk -f prog.awk data.txt > data.txt
awk -fprog.awk data.txt > data.txt
cat <<<$(jq '.dns={}' \"$cfg\") >\"$cfg\"
jq --rawfile cfg \"$cfg\" '.dns=$cfg' >\"$cfg\"
jq -Lnewmods '.x=1' \"$cfg\" >\"$cfg\"
cat < bar | gzip > bar
{ cat < baz; } > baz
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
                "foo", "foo", "foo", "data.txt", "data.txt", "data.txt", "data.txt", "\"$cfg\"",
                "\"$cfg\"", "\"$cfg\"", "\"$cfg\"", "\"$cfg\"", "\"$cfg\"", "bar", "bar", "baz",
                "baz", "\"$f\"", "\"$f\"", ".MTREE", ".MTREE", "\"$f\"", "\"$f\"",
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
sort -o out.txt in.txt > out.txt
sort --output=log.txt input.txt > log.txt
echo foo > foo
printf '%s\\n' foo > foo
cat < \"$src\" > \"$dst\"
alsamixer >/dev/tty </dev/tty
cat </dev/fd/0 >/dev/fd/0
jq --args '$ARGS.positional[0]' \"$cfg\" >\"$cfg\"
jq --jsonargs '$ARGS.positional[0]' \"$cfg\" >\"$cfg\"
jq --indent 2 --args '$ARGS.positional[0]' \"$cfg\" >\"$cfg\"
jq -nc '.x=1' \"$cfg\" >\"$cfg\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedirectClobbersInput),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_shellcheck_compatible_name_collisions() {
        let source = "\
#!/bin/bash
gzip -9c < \"$src\" > \"$pkg/usr/man/man1/$(basename \"$src\").gz\"
sort <<< \"$OUT\" > \"$OUT\"
while read iplist; do
  cat <<EOF >> json2
\"$iplist/32\"
EOF
done < iplist
(
  cat <<EOF
$SGINGRESS1
EOF
) > SGINGRESS1
{ [ \"$OUT\" -lt \"$crit_border\" ] && :; } | sort >> \"$OUT\"
{ case \"$MODE\" in on) :;; esac; } | sort > \"$MODE\"
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
                "\"$src\"",
                "\"$src\"",
                "\"$OUT\"",
                "\"$OUT\"",
                "iplist",
                "iplist",
                "iplist",
                "SGINGRESS1",
                "SGINGRESS1",
                "\"$OUT\"",
                "\"$OUT\"",
                "\"$MODE\"",
                "\"$MODE\"",
            ]
        );
    }
}
