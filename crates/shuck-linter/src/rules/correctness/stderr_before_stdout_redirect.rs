use rustc_hash::FxHashSet;
use shuck_ast::{RedirectKind, Span};

use crate::{Checker, Edit, Fix, FixAvailability, RedirectFact, Rule, Violation};

pub struct StderrBeforeStdoutRedirect;

impl Violation for StderrBeforeStdoutRedirect {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::StderrBeforeStdoutRedirect
    }

    fn message(&self) -> String {
        "stderr is redirected before stdout is redirected".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("move `2>&1` after the stdout-to-file redirects".to_owned())
    }
}

pub fn stderr_before_stdout_redirect(checker: &mut Checker) {
    let source = checker.source();
    let pipeline_producer_command_ids = checker
        .facts()
        .pipelines()
        .iter()
        .flat_map(|pipeline| {
            pipeline
                .segments()
                .split_last()
                .into_iter()
                .flat_map(|(_, producers)| producers.iter().map(|segment| segment.command_id()))
        })
        .collect::<FxHashSet<_>>();

    let diagnostics = checker
        .facts()
        .structural_commands()
        .filter(|fact| !pipeline_producer_command_ids.contains(&fact.id()))
        .flat_map(|fact| {
            let redirects = fact.redirect_facts();
            redirects
                .iter()
                .enumerate()
                .filter_map(move |(index, redirect)| {
                    if !is_stderr_to_stdout_redirect(redirect) {
                        return None;
                    }

                    let stdout_index =
                        last_later_stdout_file_redirect_index(&redirects[index + 1..])? + index + 1;
                    let diagnostic = crate::Diagnostic::new(
                        StderrBeforeStdoutRedirect,
                        redirect.redirect().span,
                    );
                    Some(
                        match stderr_before_stdout_redirect_fix(
                            source,
                            redirects,
                            index,
                            stdout_index,
                        ) {
                            Some(fix) => diagnostic.with_fix(fix),
                            None => diagnostic,
                        },
                    )
                })
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn is_stderr_to_stdout_redirect(redirect: &RedirectFact<'_>) -> bool {
    let Some(analysis) = redirect.analysis() else {
        return false;
    };

    redirect.redirect().kind == RedirectKind::DupOutput
        && redirect.redirect().fd == Some(2)
        && analysis.numeric_descriptor_target == Some(1)
}

fn stderr_before_stdout_redirect_fix(
    source: &str,
    redirects: &[RedirectFact<'_>],
    stderr_index: usize,
    stdout_index: usize,
) -> Option<Fix> {
    if redirects[stderr_index + 1..stdout_index]
        .iter()
        .any(redirect_touches_stderr)
    {
        return None;
    }

    let replacement = reordered_redirect_segment(source, redirects, stderr_index, stdout_index);
    let span = Span::from_positions(
        redirects[stderr_index].redirect().span.start,
        redirects[stdout_index].redirect().span.end,
    );

    Some(Fix::unsafe_edit(Edit::replacement(replacement, span)))
}

fn reordered_redirect_segment(
    source: &str,
    redirects: &[RedirectFact<'_>],
    stderr_index: usize,
    stdout_index: usize,
) -> String {
    let moved_span = redirects[stderr_index].redirect().span;
    let mut replacement = String::new();

    for index in stderr_index + 1..=stdout_index {
        let span = redirects[index].redirect().span;
        let gap = if index == stderr_index + 1 {
            strip_leading_shell_trivia(&source[moved_span.end.offset..span.start.offset])
        } else {
            &source[redirects[index - 1].redirect().span.end.offset..span.start.offset]
        };
        replacement.push_str(gap);
        replacement.push_str(span.slice(source));
    }

    // Always force a real separator before the moved redirect so adjacent
    // redirects and escaped newlines cannot merge the tokens back together.
    replacement.push(' ');
    replacement.push_str(moved_span.slice(source));
    replacement
}

fn strip_leading_shell_trivia(text: &str) -> &str {
    let mut offset = 0;
    let mut remaining = text;

    loop {
        if let Some(stripped) = remaining.strip_prefix("\\\r\n") {
            offset += 3;
            remaining = stripped;
            continue;
        }
        if let Some(stripped) = remaining.strip_prefix("\\\n") {
            offset += 2;
            remaining = stripped;
            continue;
        }

        let trimmed = remaining.trim_start_matches(char::is_whitespace);
        if trimmed.len() == remaining.len() {
            break;
        }
        offset += remaining.len() - trimmed.len();
        remaining = trimmed;
    }

    &text[offset..]
}

fn last_later_stdout_file_redirect_index(redirects: &[RedirectFact<'_>]) -> Option<usize> {
    redirects
        .iter()
        .enumerate()
        .filter_map(|(index, redirect)| is_stdout_file_redirect(redirect).then_some(index))
        .next_back()
}

fn is_stdout_file_redirect(redirect: &RedirectFact<'_>) -> bool {
    let data = redirect.redirect();
    data.fd.unwrap_or(1) == 1
        && matches!(
            data.kind,
            RedirectKind::Output | RedirectKind::Clobber | RedirectKind::Append
        )
}

fn redirect_touches_stderr(redirect: &RedirectFact<'_>) -> bool {
    let data = redirect.redirect();
    data.fd == Some(2) || data.kind == RedirectKind::OutputBoth
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_stdout_redirects_in_structural_commands_only() {
        let source = "\
#!/bin/sh
foo 2>&1 >/dev/null
out=$(bar 2>&1 >/dev/null)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StderrBeforeStdoutRedirect),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/sh\necho ok 2>&1 >/dev/null\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StderrBeforeStdoutRedirect),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("move `2>&1` after the stdout-to-file redirects")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_stderr_duplications_before_stdout_redirects() {
        let source = "\
#!/bin/sh
echo ok 2>&1 >/dev/null
echo ok 2>&1 3>aux >out
echo ok 2>&1 1>/dev/null
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::StderrBeforeStdoutRedirect),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
echo ok >/dev/null 2>&1
echo ok 3>aux >out 2>&1
echo ok 1>/dev/null 2>&1
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn inserts_a_separator_when_reordering_adjacent_redirect_tokens() {
        let source = "\
#!/bin/sh
echo ok 2>&1>/dev/null
echo ok 2>&1 3>aux>out
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::StderrBeforeStdoutRedirect),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
echo ok >/dev/null 2>&1
echo ok 3>aux>out 2>&1
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn inserts_a_real_separator_after_escaped_newline_redirect_gaps() {
        let source = "\
#!/bin/sh
echo ok 2>&1\\
>/tmp/out
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::StderrBeforeStdoutRedirect),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
echo ok >/tmp/out 2>&1
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn preserves_interleaved_tokens_before_the_first_later_redirect() {
        let source = "\
#!/bin/sh
echo ok 2>&1 arg >/tmp/out
echo ok 2>&1 item 3>aux >out
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::StderrBeforeStdoutRedirect),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
echo ok arg >/tmp/out 2>&1
echo ok item 3>aux >out 2>&1
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn withholds_fix_when_an_intervening_redirect_retargets_stderr() {
        let source = "\
#!/bin/sh
echo ok 2>&1 2>err >out
echo ok 2>&1 &>out >final
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StderrBeforeStdoutRedirect),
        );

        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_none())
        );

        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::StderrBeforeStdoutRedirect),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(
            result
                .fixed_diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_none())
        );
    }

    #[test]
    fn leaves_non_matching_redirect_orders_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
foo 2>&1 >/dev/null | sed 's/x/y/'
echo ok >file 2>&1
echo ok 2>&1 1>&3
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::StderrBeforeStdoutRedirect),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C085.sh").as_path(),
            &LinterSettings::for_rule(Rule::StderrBeforeStdoutRedirect),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C085_fix_C085.sh", result);
        Ok(())
    }

    #[test]
    fn ignores_pipeline_producers_but_keeps_pipeline_tail_reports() {
        let source = "\
#!/bin/sh
foo 2>&1 >/dev/null | sed 's/x/y/'
echo ok | foo 2>&1 >/dev/null
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StderrBeforeStdoutRedirect),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
    }
}
