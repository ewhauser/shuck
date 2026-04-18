use shuck_ast::{RedirectKind, Span};

use crate::{
    Checker, ExpansionContext, PipelineFact, Rule, ShellDialect, Violation, WordFact,
    WordFactContext, word_double_quoted_scalar_only_expansion_spans,
    word_quoted_all_elements_array_slice_spans,
};

pub struct EchoToSedSubstitution;

impl Violation for EchoToSedSubstitution {
    fn rule() -> Rule {
        Rule::EchoToSedSubstitution
    }

    fn message(&self) -> String {
        "prefer a shell rewrite over piping echo into sed for one substitution".to_owned()
    }
}

pub fn echo_to_sed_substitution(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Bash | ShellDialect::Ksh) {
        return;
    }

    let spans = checker
        .facts()
        .pipelines()
        .iter()
        .filter_map(|pipeline| sc2001_like_pipeline_span(checker, pipeline))
        .chain(
            checker
                .facts()
                .commands()
                .iter()
                .filter_map(|command| sc2001_like_here_string_span(command, checker.source())),
        )
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || EchoToSedSubstitution);
}

fn sc2001_like_pipeline_span(checker: &Checker<'_>, pipeline: &PipelineFact<'_>) -> Option<Span> {
    let [left_segment, right_segment] = pipeline.segments() else {
        return None;
    };

    let left = checker.facts().command(left_segment.command_id());
    let right = checker.facts().command(right_segment.command_id());

    if !is_plain_command_named(left, "echo") || !is_plain_command_named(right, "sed") {
        return None;
    }

    if left
        .options()
        .echo()
        .and_then(|echo| echo.portability_flag_word())
        .is_some()
    {
        return None;
    }

    if !right
        .options()
        .sed()
        .is_some_and(|sed| sed.has_single_substitution_script())
    {
        return None;
    }

    let [argument] = left.body_args() else {
        return None;
    };

    let word_fact = checker.facts().word_fact(
        argument.span,
        WordFactContext::Expansion(ExpansionContext::CommandArgument),
    )?;

    if word_fact.static_text().is_some() {
        return None;
    }

    if word_fact.scalar_expansion_spans().is_empty()
        && word_fact.array_expansion_spans().is_empty()
        && word_fact.command_substitution_spans().is_empty()
    {
        return None;
    }

    if word_fact.has_literal_affixes() && !is_pure_quoted_dynamic_word(word_fact, checker.source())
    {
        return None;
    }

    Some(pipeline_span(checker, pipeline))
}

fn pipeline_span(checker: &Checker<'_>, pipeline: &PipelineFact<'_>) -> Span {
    let source = checker.source();
    let first = checker.facts().command(
        pipeline
            .first_segment()
            .expect("pipeline has segments")
            .command_id(),
    );
    let last = checker.facts().command(
        pipeline
            .last_segment()
            .expect("pipeline has segments")
            .command_id(),
    );
    let last_end = last.span_in_source(source).end;
    let end = extend_over_trailing_inline_space(last_end, source);

    Span::from_positions(
        first
            .body_name_word()
            .expect("plain echo command should have a body name")
            .span
            .start,
        end,
    )
}

fn is_plain_command_named(fact: &crate::CommandFact<'_>, name: &str) -> bool {
    fact.effective_name_is(name) && fact.wrappers().is_empty()
}

fn sc2001_like_here_string_span(command: &crate::CommandFact<'_>, source: &str) -> Option<Span> {
    if !is_plain_command_named(command, "sed") {
        return None;
    }

    if !command
        .options()
        .sed()
        .is_some_and(|sed| sed.has_single_substitution_script())
    {
        return None;
    }

    let mut here_strings = command
        .redirect_facts()
        .iter()
        .filter(|redirect| redirect.redirect().kind == RedirectKind::HereString);
    here_strings.next()?;
    if here_strings.next().is_some() {
        return None;
    }

    command_with_redirects_span(command, source)
}

fn command_with_redirects_span(command: &crate::CommandFact<'_>, source: &str) -> Option<Span> {
    let body_name = command.body_name_word()?;
    let mut end = body_name.span.end;

    for word in command.body_args() {
        if word.span.end.offset > end.offset {
            end = word.span.end;
        }
    }

    for redirect in command.redirect_facts() {
        let redirect_end = redirect.redirect().span.end;
        if redirect_end.offset > end.offset {
            end = redirect_end;
        }
    }

    Some(Span::from_positions(
        body_name.span.start,
        extend_over_trailing_inline_space(end, source),
    ))
}

fn extend_over_trailing_inline_space(
    end: shuck_ast::Position,
    source: &str,
) -> shuck_ast::Position {
    let tail = &source[end.offset..];
    let spaces_len = tail
        .char_indices()
        .take_while(|(_, ch)| matches!(ch, ' ' | '\t'))
        .last()
        .map_or(0, |(index, ch)| index + ch.len_utf8());

    if spaces_len == 0 {
        return end;
    }

    let rest = &tail[spaces_len..];
    if rest.is_empty()
        || rest.starts_with('\n')
        || rest.starts_with('\r')
        || rest.starts_with(')')
        || rest.starts_with(']')
        || rest.starts_with('}')
    {
        end.advanced_by(&tail[..spaces_len])
    } else {
        end
    }
}

fn is_pure_quoted_dynamic_word(fact: &WordFact<'_>, source: &str) -> bool {
    !word_double_quoted_scalar_only_expansion_spans(fact.word()).is_empty()
        || !word_quoted_all_elements_array_slice_spans(fact.word()).is_empty()
        || is_double_quoted_command_substitution_only(fact, source)
        || is_backtick_escaped_double_quoted_dynamic_word(fact, source)
}

fn is_double_quoted_command_substitution_only(fact: &WordFact<'_>, source: &str) -> bool {
    let [command_substitution] = fact.command_substitution_spans() else {
        return false;
    };

    if !fact.scalar_expansion_spans().is_empty() || !fact.array_expansion_spans().is_empty() {
        return false;
    }

    let word_text = fact.span().slice(source);
    word_text.len() == command_substitution.slice(source).len() + 2
        && word_text.starts_with('"')
        && word_text.ends_with('"')
        && &word_text[1..word_text.len() - 1] == command_substitution.slice(source)
}

fn is_backtick_escaped_double_quoted_dynamic_word(fact: &WordFact<'_>, source: &str) -> bool {
    let word_text = fact.span().slice(source);
    if !word_text.starts_with("\\\"") || !word_text.ends_with("\\\"") {
        return false;
    }

    let inner = &word_text[2..word_text.len() - 2];
    match (
        fact.scalar_expansion_spans(),
        fact.array_expansion_spans(),
        fact.command_substitution_spans(),
    ) {
        ([scalar], [], []) => inner == scalar.slice(source),
        ([], [array], []) => inner == array.slice(source),
        ([], [], [command_substitution]) => inner == command_substitution.slice(source),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_plain_echo_to_sed_rewrites() {
        let source = "\
#!/bin/bash
echo $value | sed 's/foo/bar/'
echo \"$value\" | sed 's/foo/bar/g'
echo ${items[@]} | sed -e 's/foo/bar/2'
result=$(echo \"$(printf %s foo)\" | sed 's/foo/bar/')
COMMAND=$(echo \"$COMMAND\" | sed \"s#\\(--appendconfig *[^ $]*\\)#\\1'|'$conf#\")
RUNTIME=$(echo $OUT | sed \"s|$OUT|\\$this_dir|g\")
escaped_hostname=$(echo \"$hostname\" | sed 's/[]\\[\\.^$*+?{}()|]/\\\\&/g')
value=$(sed 's/[\\.|$(){}?+*^]/\\\\&/g' <<<\"$value\")
CFLAGS=\"`echo \\\"$CFLAGS\\\" | sed \\\"s/ $COVERAGE_FLAGS//\\\"`\"
OPTFLAG=\"`echo \\\"$CFLAGS\\\" | sed 's/^.*\\(-O[^ ]\\).*$/\\1/'`\"
EC2_REGION=\"`echo \\\"$EC2_AVAIL_ZONE\\\" | sed -e 's:\\([0-9][0-9]*\\)[a-z]*\\$:\\1:'`\"
echo \"$caps_add\" | sed 's/^/  /' \t
trimmed=$(sed 's/[[:space:]]*$//' <<<\"$value\")
literal=$(sed 's/[[:space:]]*$//' <<<literal)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoToSedSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "echo $value | sed 's/foo/bar/'",
                "echo \"$value\" | sed 's/foo/bar/g'",
                "echo ${items[@]} | sed -e 's/foo/bar/2'",
                "echo \"$(printf %s foo)\" | sed 's/foo/bar/'",
                "echo \"$COMMAND\" | sed \"s#\\(--appendconfig *[^ $]*\\)#\\1'|'$conf#\"",
                "echo $OUT | sed \"s|$OUT|\\$this_dir|g\"",
                "echo \"$hostname\" | sed 's/[]\\[\\.^$*+?{}()|]/\\\\&/g'",
                "sed 's/[\\.|$(){}?+*^]/\\\\&/g' <<<\"$value\"",
                "echo \\\"$CFLAGS\\\" | sed \\\"s/ $COVERAGE_FLAGS//\\\"",
                "echo \\\"$CFLAGS\\\" | sed 's/^.*\\(-O[^ ]\\).*$/\\1/'",
                "echo \\\"$EC2_AVAIL_ZONE\\\" | sed -e 's:\\([0-9][0-9]*\\)[a-z]*\\$:\\1:'",
                "echo \"$caps_add\" | sed 's/^/  /' \t",
                "sed 's/[[:space:]]*$//' <<<\"$value\"",
                "sed 's/[[:space:]]*$//' <<<literal",
            ]
        );
    }

    #[test]
    fn ignores_nonmatching_echo_shapes_and_sed_forms() {
        let source = "\
#!/bin/bash
echo literal | sed 's/foo/bar/'
echo prefix${value}suffix | sed 's/foo/bar/'
echo \"prefix${value}\" | sed 's/foo/bar/'
echo $left $right | sed 's/foo/bar/'
echo \"$left $right\" | sed 's/foo/bar/'
echo -n $value | sed 's/foo/bar/'
echo $value | sed -n 's/foo/bar/p'
echo $value | sed --expression 's/foo/bar/'
echo $value | sed -es/foo/bar/
echo $value | sed 's/foo/bar/' | cat
echo \"$ENDPOINT\" | sed 's/[:\\/]/_/g'
echo \"$PAYLOAD\" | sed 's/\\//-/g'
echo $PACKAGE_NAME | sed 's/\\./\\//g'
echo \"$key\" | sed 's/[]\\[^$.*/]/\\\\&/g'
echo \"${ENTRY}\" | sed 's/\\([/&]\\)/\\\\\\1/g'
sed 's/[]\\[^$.*/]/\\\\&/g' <<<\"$key\"
sed 's/\\([/&]\\)/\\\\\\1/g' <<<\"${ENTRY}\"
printf '%s\\n' \"$value\" | sed 's/foo/bar/'
echo \"prefix$(printf %s foo)\" | sed 's/foo/bar/'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoToSedSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_plain_sh_scripts() {
        let source = "\
#!/bin/sh
echo $value | sed 's/foo/bar/'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoToSedSubstitution).with_shell(ShellDialect::Sh),
        );

        assert!(diagnostics.is_empty());
    }
}
