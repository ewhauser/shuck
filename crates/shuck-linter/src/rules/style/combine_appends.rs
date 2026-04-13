use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{RedirectKind, Span};

use crate::{
    Checker, ComparablePathKey, ExpansionContext, FactSpan, Rule, StatementFact, Violation,
    comparable_path,
};

pub struct CombineAppends;

impl Violation for CombineAppends {
    fn rule() -> Rule {
        Rule::CombineAppends
    }

    fn message(&self) -> String {
        "multiple commands append to the same file; use one grouped redirect".to_owned()
    }
}

pub fn combine_appends(checker: &mut Checker) {
    let source = checker.source();
    let mut bodies: FxHashMap<FactSpan, Vec<StatementFact>> = FxHashMap::default();
    let case_item_body_spans = checker
        .facts()
        .case_items()
        .iter()
        .map(|item| item.item().body.span)
        .collect::<Vec<_>>();

    for fact in checker.facts().statement_facts() {
        if case_item_body_spans.iter().any(|span| {
            fact.stmt_span().start.offset >= span.start.offset
                && fact.stmt_span().end.offset <= span.end.offset
        }) {
            continue;
        }

        bodies
            .entry(FactSpan::new(fact.body_span()))
            .or_default()
            .push(*fact);
    }

    let spans = bodies
        .into_values()
        .flat_map(|mut statements| append_run_spans_in_body(checker, &mut statements, source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || CombineAppends);
}

fn append_run_spans_in_body(
    checker: &Checker<'_>,
    statements: &mut [StatementFact],
    source: &str,
) -> Vec<Span> {
    statements.sort_by_key(|fact| fact.stmt_span().start.offset);

    let mut spans = Vec::new();
    let mut current_key: Option<ComparablePathKey> = None;
    let mut current_run_len = 0usize;
    let mut current_first_span = None;

    for statement in statements.iter() {
        let Some((key, span)) = append_target_for_statement(checker, statement, source) else {
            if current_run_len >= 3
                && let Some(first_span) = current_first_span.take()
            {
                spans.push(first_span);
            }
            current_key = None;
            current_run_len = 0;
            continue;
        };

        match &current_key {
            Some(existing) if *existing == key => {
                current_run_len += 1;
            }
            _ => {
                if current_run_len >= 3
                    && let Some(first_span) = current_first_span.take()
                {
                    spans.push(first_span);
                }
                current_key = Some(key);
                current_run_len = 1;
                current_first_span = Some(span);
            }
        }
    }

    if current_run_len >= 3
        && let Some(first_span) = current_first_span.take()
    {
        spans.push(first_span);
    }

    spans
}

fn append_target_for_statement(
    checker: &Checker<'_>,
    statement: &StatementFact,
    source: &str,
) -> Option<(ComparablePathKey, Span)> {
    let command = checker.facts().command(statement.command_id());
    let statement_commands = commands_for_statement(checker, statement);
    let mut target: Option<ComparablePathKey> = None;
    let mut anchor_end = command.body_span().end;

    for command in statement_commands {
        if command.body_span().end.offset > anchor_end.offset {
            anchor_end = command.body_span().end;
        }

        for redirect in command.redirect_facts() {
            if redirect.redirect().kind != RedirectKind::Append {
                continue;
            }

            let analysis = redirect.analysis()?;
            if !analysis.is_file_target() {
                continue;
            }

            let target_word = redirect.redirect().word_target()?;
            let comparable = comparable_path(
                target_word,
                source,
                ExpansionContext::RedirectTarget(RedirectKind::Append),
                command.zsh_options(),
            )?;
            let key = comparable.key().clone();
            if comparable.span().end.offset > anchor_end.offset {
                anchor_end = comparable.span().end;
            }

            match &target {
                Some(existing) if *existing == key => {}
                Some(_) => return None,
                None => target = Some(key),
            }
        }
    }

    target.map(|key| (key, Span::from_positions(command.body_span().start, anchor_end)))
}

fn commands_for_statement<'a>(
    checker: &'a Checker<'_>,
    statement: &StatementFact,
) -> Vec<&'a crate::CommandFact<'a>> {
    let command = checker.facts().command(statement.command_id());
    let mut command_ids = FxHashSet::default();
    let mut commands = Vec::new();

    command_ids.insert(statement.command_id());
    commands.push(command);

    if let Some(pipeline) = checker
        .facts()
        .pipelines()
        .iter()
        .find(|pipeline| pipeline.span() == command.span())
    {
        for segment in pipeline.segments() {
            if command_ids.insert(segment.command_id()) {
                commands.push(checker.facts().command(segment.command_id()));
            }
        }
    }

    commands
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_first_command_in_three_command_append_runs() {
        let source = "\
#!/bin/sh
echo one >> out.log
echo two >> out.log
echo three >> out.log
echo first >> semi.log; echo second >> semi.log; echo third >> semi.log
echo alpha >> \"$log\"
echo beta >> \"$log\"
echo gamma >> \"$log\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CombineAppends));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "echo one >> out.log",
                "echo first >> semi.log",
                "echo alpha >> \"$log\"",
            ]
        );
    }

    #[test]
    fn ignores_two_command_runs_and_already_grouped_redirects() {
        let source = "\
#!/bin/sh
echo one >> out.log
echo two >> out.log
{ echo group one; } >> grouped.log
{ echo group two; } >> grouped.log
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CombineAppends));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn counts_pipeline_statements_and_trims_inline_comment_anchors() {
        let source = "\
#!/bin/sh
echo \"pre_install() {\" >> .INSTALL # comment
cat preinst | grep -v '^#' >> .INSTALL
echo \"}\" >> .INSTALL
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CombineAppends));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo \"pre_install() {\" >> .INSTALL"]
        );
    }

    #[test]
    fn ignores_append_runs_inside_case_arms() {
        let source = "\
#!/bin/sh
case \"$kind\" in
  kernel)
    echo one >> out.log
    echo two >> out.log
    echo three >> out.log
    ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CombineAppends));

        assert!(diagnostics.is_empty());
    }
}
