use shuck_ast::Span;

use crate::{
    Checker, CommandFactRef, Edit, Fix, FixAvailability, PipelineFact, PipelineSegmentFact, Rule,
    Violation,
};

pub struct FindOutputToXargs;

impl Violation for FindOutputToXargs {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::FindOutputToXargs
    }

    fn message(&self) -> String {
        "raw `find` output piped to `xargs` can break on whitespace".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("make the `find | xargs` handoff NUL-delimited".to_owned())
    }
}

pub fn find_output_to_xargs(checker: &mut Checker) {
    let diagnostics = checker
        .facts()
        .pipelines()
        .iter()
        .flat_map(|pipeline| unsafe_find_to_xargs_diagnostics(checker, pipeline))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn unsafe_find_to_xargs_diagnostics(
    checker: &Checker<'_>,
    pipeline: &PipelineFact,
) -> Vec<crate::Diagnostic> {
    pipeline
        .segments()
        .windows(2)
        .filter_map(|pair| {
            let left_segment = &pair[0];
            let right_segment = &pair[1];
            let left = checker.facts().command(left_segment.command_id());
            let right = checker.facts().command(right_segment.command_id());

            if !left_segment.effective_name_is("find") || !right_segment.effective_name_is("xargs")
            {
                return None;
            }

            if left
                .options()
                .find()
                .is_some_and(|find| find.has_formatted_output_action())
            {
                return None;
            }

            if right
                .options()
                .xargs()
                .is_some_and(|xargs| xargs.uses_null_input || xargs.has_zero_digit_option_word())
            {
                return None;
            }

            let span = find_command_span(left_segment, left, checker.source());
            let fix = find_output_to_xargs_fix(left, right, checker.source());

            Some(crate::Diagnostic::new(FindOutputToXargs, span).with_fix(fix))
        })
        .collect()
}

fn find_output_to_xargs_fix(
    left: CommandFactRef<'_, '_>,
    right: CommandFactRef<'_, '_>,
    source: &str,
) -> Fix {
    let mut edits = Vec::new();

    if !left.options().find().is_some_and(|find| find.has_print0) {
        edits.push(Edit::insertion(
            find_print0_insertion_offset(left, source),
            " -print0",
        ));
    }

    if !right
        .options()
        .xargs()
        .is_some_and(|xargs| xargs.uses_null_input)
    {
        edits.push(Edit::insertion(
            xargs_null_input_insertion_offset(right, source),
            " -0",
        ));
    }

    debug_assert!(
        !edits.is_empty(),
        "fixable find | xargs diagnostics should add at least one edit"
    );
    Fix::unsafe_edits(edits)
}

fn find_print0_insertion_offset(command: CommandFactRef<'_, '_>, source: &str) -> usize {
    command
        .redirect_facts()
        .first()
        .map(|redirect| redirect.span().start.offset)
        .or_else(|| {
            command
                .arena_body_args(source)
                .last()
                .map(|word| word.span().end.offset)
        })
        .or_else(|| {
            command
                .arena_body_name_word(source)
                .map(|word| word.span().end.offset)
        })
        .expect("find command diagnostics should have a body insertion point")
}

fn xargs_null_input_insertion_offset(command: CommandFactRef<'_, '_>, source: &str) -> usize {
    command
        .arena_body_name_word(source)
        .map(|word| word.span().end.offset)
        .expect("xargs command diagnostics should have a body name word")
}

fn find_command_span(
    _segment: &PipelineSegmentFact,
    fact: CommandFactRef<'_, '_>,
    source: &str,
) -> Span {
    let end = fact
        .redirect_facts()
        .last()
        .map(|redirect| redirect.span().end)
        .or_else(|| fact.arena_body_args(source).last().map(|word| word.span().end))
        .or_else(|| fact.body_word_span().map(|span| span.end))
        .unwrap_or_else(|| fact.body_span().end);
    Span::from_positions(fact.body_span().start, end)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn anchors_on_effective_find_command_name() {
        let source = "command find . -type f | xargs wc -l\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputToXargs));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "find . -type f");
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("make the `find | xargs` handoff NUL-delimited")
        );
    }

    #[test]
    fn anchors_on_multiline_find_segment_before_pipe() {
        let source = "find . -type f \\\n  -name '*.txt' | xargs rm\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputToXargs));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "find . -type f \\\n  -name '*.txt'"
        );
    }

    #[test]
    fn accepts_null_delimited_find_xargs_pairs_and_reports_wrapped_find() {
        let source = "\
find . -type f -print0 | xargs -0 rm
command find . -type f | xargs rm
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputToXargs));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "find . -type f");
    }

    #[test]
    fn ignores_xargs_null_mode_even_when_find_does_not_use_print0() {
        let source = "\
find \"$pkg\" -print1 | xargs -0 file
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputToXargs));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn follows_shellcheck_zero_digit_xargs_option_exemption() {
        let source = "\
find \"$dir\" \\( -type f -o -type l \\) -and -not -path \"$dir/plugins/*\" | xargs -I % -P10 bash -c '. /tmp/lib.sh && foo %'
find . -type f | xargs -P0 rm
find . -type f | xargs -P 10 rm
find . -type f | xargs --max-procs=10 rm
find . -type f -print0 | xargs -P10 rm
find . -type f | xargs -P11 rm
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputToXargs));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "find . -type f");
        assert_eq!(diagnostics[1].span.slice(source), "find . -type f");
    }

    #[test]
    fn reports_replacement_xargs_without_null_input() {
        let source = "\
find \"$dir\" \\( -type f -o -type l \\) -and -not -path \"$dir/plugins/*\" | xargs -I % -P1 bash -c '. /tmp/lib.sh && foo %'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputToXargs));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "find \"$dir\" \\( -type f -o -type l \\) -and -not -path \"$dir/plugins/*\""
        );
    }

    #[test]
    fn ignores_find_printf_output_actions_but_reports_print0_without_null_xargs() {
        let source = "\
find plugins/ -maxdepth 2 -name '__init__.py' -printf '%h\\n' | xargs mv -t \"$dest\"
find \"$pkg\" -print0 | xargs rm
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputToXargs));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "find \"$pkg\" -print0");
    }

    #[test]
    fn applies_unsafe_fix_to_find_xargs_pairs_missing_null_handoff_flags() {
        let source = "\
#!/bin/sh
find . -name '*.txt' | xargs rm
find . -type f | xargs -0 wc -l
find \"$pkg\" -print0 | xargs rm
command find . -type f | command xargs wc -l
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::FindOutputToXargs),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
find . -name '*.txt' -print0 | xargs -0 rm
find . -type f | xargs -0 wc -l
find \"$pkg\" -print0 | xargs -0 rm
command find . -type f -print0 | command xargs -0 wc -l
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_formatted_find_output_and_safe_pairs_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
find plugins/ -maxdepth 2 -name '__init__.py' -printf '%h\\n' | xargs mv -t \"$dest\"
find . -name '*.txt' -print0 | xargs -0 rm
printf '%s\\n' ./a ./b | xargs rm
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::FindOutputToXargs),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C007.sh").as_path(),
            &LinterSettings::for_rule(Rule::FindOutputToXargs),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C007_fix_C007.sh", result);
        Ok(())
    }
}
