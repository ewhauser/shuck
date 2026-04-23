use rustc_hash::FxHashMap;
use shuck_ast::{RedirectKind, Span};

use crate::{
    Checker, ComparableNameKey, ComparableNameUseKind, ComparablePathKey, ComparablePathMatchKey,
    ExpansionContext, Rule, Violation,
};

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

    let mut read_paths: FxHashMap<RedirectPathMatchKey, Vec<Span>> = FxHashMap::default();
    let mut write_paths: FxHashMap<RedirectPathMatchKey, Vec<Span>> = FxHashMap::default();
    let mut readwrite_paths: FxHashMap<RedirectPathMatchKey, Vec<Span>> = FxHashMap::default();
    let mut input_read_names: FxHashMap<ComparableNameKey, Vec<Span>> = FxHashMap::default();
    let mut literal_input_read_names: FxHashMap<ComparableNameKey, Vec<Span>> =
        FxHashMap::default();
    let mut parameter_input_read_names: FxHashMap<ComparableNameKey, Vec<Span>> =
        FxHashMap::default();
    let mut heredoc_read_names: FxHashMap<ComparableNameKey, Vec<Span>> = FxHashMap::default();
    let mut literal_write_names: FxHashMap<ComparableNameKey, Vec<Span>> = FxHashMap::default();
    let mut derived_write_names: FxHashMap<ComparableNameKey, Vec<Span>> = FxHashMap::default();
    let mut literal_read_target_write_names: FxHashMap<ComparableNameKey, Vec<Span>> =
        FxHashMap::default();
    let mut parameter_read_target_write_names: FxHashMap<ComparableNameKey, Vec<Span>> =
        FxHashMap::default();
    let own_readwrite_spans = fact
        .redirect_facts()
        .iter()
        .filter(|redirect| redirect.redirect().kind == RedirectKind::ReadWrite)
        .filter_map(|redirect| redirect.redirect().word_target().map(|word| word.span))
        .collect::<Vec<_>>();

    for redirect in fact.redirect_facts() {
        if matches!(
            redirect.redirect().kind,
            RedirectKind::Output
                | RedirectKind::Clobber
                | RedirectKind::Append
                | RedirectKind::OutputBoth
        ) {
            for name_use in redirect.comparable_name_uses() {
                match name_use.kind() {
                    ComparableNameUseKind::Literal => {
                        literal_write_names
                            .entry(name_use.key().clone())
                            .or_default()
                            .push(name_use.span());
                    }
                    ComparableNameUseKind::Derived => {
                        derived_write_names
                            .entry(name_use.key().clone())
                            .or_default()
                            .push(name_use.span());
                    }
                    ComparableNameUseKind::Parameter => {}
                }
            }
        }

        let Some(comparable) = redirect.comparable_path() else {
            continue;
        };

        let key = redirect_path_match_key(comparable.key(), comparable.match_key());
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
            .entry(redirect_path_match_key(
                comparable.key(),
                comparable.match_key(),
            ))
            .or_default()
            .push(comparable.span());
    }

    for name_use in fact.scope_name_read_uses() {
        input_read_names
            .entry(name_use.key().clone())
            .or_default()
            .push(name_use.span());
        match name_use.kind() {
            ComparableNameUseKind::Literal => {
                literal_input_read_names
                    .entry(name_use.key().clone())
                    .or_default()
                    .push(name_use.span());
            }
            ComparableNameUseKind::Parameter => {
                parameter_input_read_names
                    .entry(name_use.key().clone())
                    .or_default()
                    .push(name_use.span());
            }
            ComparableNameUseKind::Derived => {}
        }
    }

    for name_use in fact.scope_heredoc_name_read_uses() {
        heredoc_read_names
            .entry(name_use.key().clone())
            .or_default()
            .push(name_use.span());
    }

    for name_use in fact.scope_name_write_uses() {
        match name_use.kind() {
            ComparableNameUseKind::Literal => {
                literal_read_target_write_names
                    .entry(name_use.key().clone())
                    .or_default()
                    .push(name_use.span());
            }
            ComparableNameUseKind::Parameter => {
                parameter_read_target_write_names
                    .entry(name_use.key().clone())
                    .or_default()
                    .push(name_use.span());
            }
            ComparableNameUseKind::Derived => {}
        }
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

    for (key, read_spans) in &input_read_names {
        let Some(write_spans) = derived_write_names.get(key) else {
            continue;
        };

        spans.extend(read_spans.iter().copied());
        spans.extend(write_spans.iter().copied());
    }

    extend_matching_name_spans(
        &literal_input_read_names,
        &literal_read_target_write_names,
        &mut spans,
    );
    extend_matching_name_spans(
        &literal_input_read_names,
        &parameter_read_target_write_names,
        &mut spans,
    );
    extend_matching_name_spans(
        &parameter_input_read_names,
        &parameter_read_target_write_names,
        &mut spans,
    );

    for (key, read_spans) in &heredoc_read_names {
        let Some(write_spans) = literal_write_names.get(key) else {
            continue;
        };

        spans.extend(read_spans.iter().copied());
        spans.extend(write_spans.iter().copied());
    }

    for (key, read_spans) in &heredoc_read_names {
        if !has_name_write_signal(
            key,
            &literal_write_names,
            &derived_write_names,
            &literal_read_target_write_names,
            &parameter_read_target_write_names,
        ) {
            continue;
        }

        let Some(input_spans) = input_read_names.get(key) else {
            continue;
        };

        spans.extend(read_spans.iter().copied());
        spans.extend(input_spans.iter().copied());
    }

    spans
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RedirectPathMatchKey {
    Literal(ComparablePathKey),
    Exact(ComparablePathMatchKey),
}

fn redirect_path_match_key(
    key: &ComparablePathKey,
    exact: ComparablePathMatchKey,
) -> RedirectPathMatchKey {
    match key {
        ComparablePathKey::Literal(_) => RedirectPathMatchKey::Literal(key.clone()),
        ComparablePathKey::Parameter(_) | ComparablePathKey::Template(_) => {
            RedirectPathMatchKey::Exact(exact)
        }
    }
}

fn has_name_write_signal(
    key: &ComparableNameKey,
    literal_write_names: &FxHashMap<ComparableNameKey, Vec<Span>>,
    derived_write_names: &FxHashMap<ComparableNameKey, Vec<Span>>,
    literal_read_target_write_names: &FxHashMap<ComparableNameKey, Vec<Span>>,
    parameter_read_target_write_names: &FxHashMap<ComparableNameKey, Vec<Span>>,
) -> bool {
    literal_write_names.contains_key(key)
        || derived_write_names.contains_key(key)
        || literal_read_target_write_names.contains_key(key)
        || parameter_read_target_write_names.contains_key(key)
}

fn extend_matching_name_spans(
    left: &FxHashMap<ComparableNameKey, Vec<Span>>,
    right: &FxHashMap<ComparableNameKey, Vec<Span>>,
    spans: &mut Vec<Span>,
) {
    for (key, left_spans) in left {
        let Some(right_spans) = right.get(key) else {
            continue;
        };

        spans.extend(left_spans.iter().copied());
        spans.extend(right_spans.iter().copied());
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
    fn reports_literal_redirect_paths_across_quote_forms() {
        let source = "\
#!/bin/bash
cat < \"foo\" > foo
cat < bar > 'bar'
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
            vec!["\"foo\"", "foo", "bar", "'bar'"]
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
cat < $src > \"$src\"
cat < \"$src\" > $src
{ [ \"$OUT\" = \"0\" ]; } >>$OUT
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedirectClobbersInput),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_heredoc_and_input_name_reuse_without_a_write_signal() {
        let source = "\
#!/bin/bash
cat <<EOF < foo
$foo
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedirectClobbersInput),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_shellcheck_compatible_name_reuse_patterns() {
        let source = "\
#!/bin/bash
while read iplist; do :; done < iplist
read -r iplist < iplist
{ cat <<EOT
$SGINGRESS1
EOT
} > SGINGRESS1
gzip -9c < \"$file\" > \"$(basename \"$file\").gz\"
while read name; do cat <<EOT2
$name
EOT2
done < name
{ [ $OUT -lt 1 ]; } >>$OUT
{ [ \"$OUT\" = \"0\" ]; } >>\"$OUT\"
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
                "iplist",
                "iplist",
                "iplist",
                "iplist",
                "SGINGRESS1",
                "SGINGRESS1",
                "\"$file\"",
                "\"$file\"",
                "name",
                "name",
                "name",
                "$OUT",
                "$OUT",
                "\"$OUT\"",
                "\"$OUT\"",
            ]
        );
    }

    #[test]
    fn ignores_name_reuse_when_oracle_keeps_derived_paths_quiet() {
        let source = "\
#!/bin/bash
cat < \"$file\" > \"$file.bak\"
cat < \"$file\" > \"out/$file\"
cat < \"$dir/in\" > \"$dir/out\"
cat \"$file\" > \"$(basename \"$file\").gz\"
while read line; do :; done < iplist
while read -r x; do :; done < <(for x in \"$root\"; do find \"$x\"; done)
read linkdest < \"$linkdest\"
cat > \"$PRGNAM\" <<EOF2
$PRGNAM
EOF2
for i in man/*.1; do gzip -9c < $i > $PKGMAN1/$(basename \"$i\").gz; done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedirectClobbersInput),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_trailing_read_names_after_array_targets() {
        let source = "\
#!/bin/bash
read -a arr name < name
read -aarr name < name
read -ar name < name
read -a arr < arr
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
            vec!["arr", "arr"]
        );
    }

    #[test]
    fn reports_quoted_read_targets_that_match_input_paths() {
        let source = "\
#!/bin/bash
read \"path\" < path
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
            vec!["\"path\"", "path"]
        );
    }

    #[test]
    fn reports_quoted_literal_redirect_targets_that_match_heredoc_names() {
        let source = "\
#!/bin/bash
{ cat <<EOF
$SGINGRESS1
EOF
} > \"SGINGRESS1\"
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
            vec!["SGINGRESS1", "\"SGINGRESS1\""]
        );
    }

    #[test]
    fn ignores_quoted_input_redirect_names_for_read_targets() {
        let source = "\
#!/bin/bash
read -r KALUA_REPO_URL <'KALUA_REPO_URL'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedirectClobbersInput),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_plain_names_that_look_like_special_devices() {
        let source = "\
#!/bin/bash
read -r stdin < stdin
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
            vec!["stdin", "stdin"]
        );
    }
}
