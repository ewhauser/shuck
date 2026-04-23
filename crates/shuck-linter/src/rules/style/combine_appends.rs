use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{RedirectKind, Span};

use crate::{Checker, ComparablePathKey, FactSpan, Rule, StatementFact, Violation};

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
    let mut bodies: FxHashMap<FactSpan, Vec<StatementFact>> = FxHashMap::default();
    let case_item_body_spans = checker
        .facts()
        .case_items()
        .iter()
        .map(|item| FactSpan::new(item.item().body.span))
        .collect::<FxHashSet<_>>();

    for fact in checker.facts().statement_facts() {
        if case_item_body_spans.contains(&FactSpan::new(fact.body_span())) {
            continue;
        }

        bodies
            .entry(FactSpan::new(fact.body_span()))
            .or_default()
            .push(*fact);
    }

    let spans = bodies
        .into_values()
        .flat_map(|mut statements| append_run_spans_in_body(checker, &mut statements))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || CombineAppends);
}

fn append_run_spans_in_body(checker: &Checker<'_>, statements: &mut [StatementFact]) -> Vec<Span> {
    statements.sort_by_key(|fact| fact.stmt_span().start.offset);

    let mut spans = Vec::new();
    let mut current_key: Option<ComparablePathKey> = None;
    let mut current_run_len = 0usize;
    let mut current_first_span = None;

    for statement in statements.iter() {
        let Some((key, span)) = append_target_for_statement(checker, statement) else {
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
) -> Option<(ComparablePathKey, Span)> {
    let statement_commands = commands_for_statement(checker, statement);
    let mut target: Option<ComparablePathKey> = None;
    let mut anchor_start = None;
    let mut anchor_end = None;

    for command in statement_commands {
        for redirect in command.redirect_facts() {
            if redirect.redirect().kind != RedirectKind::Append {
                continue;
            }

            let analysis = redirect.analysis()?;
            if !analysis.is_file_target() {
                continue;
            }

            let comparable = redirect.comparable_path()?;
            let key = comparable.key().clone();
            if anchor_start.is_none() {
                let command_end = command
                    .body_args()
                    .last()
                    .map(|word| word.span.end)
                    .or_else(|| command.body_word_span().map(|span| span.end))
                    .unwrap_or(command.body_span().end);
                let redirect_end = command
                    .redirect_facts()
                    .iter()
                    .map(|redirect| redirect.redirect().span.end)
                    .max_by_key(|position| position.offset)
                    .unwrap_or(command_end);
                anchor_start = Some(command.body_span().start);
                anchor_end = Some(if redirect_end.offset > command_end.offset {
                    redirect_end
                } else {
                    command_end
                });
            }
            if let Some(current_end) = &mut anchor_end
                && comparable.span().end.offset > current_end.offset
            {
                *current_end = comparable.span().end;
            }

            match &target {
                Some(existing) if *existing == key => {}
                Some(_) => return None,
                None => target = Some(key),
            }
        }
    }

    match (target, anchor_start, anchor_end) {
        (Some(key), Some(anchor_start), Some(anchor_end)) => {
            Some((key, Span::from_positions(anchor_start, anchor_end)))
        }
        (Some(_), _, _) => None,
        (None, _, _) => None,
    }
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
    fn reports_append_runs_inside_loop_bodies() {
        let source = "\
#!/bin/sh
for js in one; do
    echo first >> \"$js\"
    echo second >> \"$js\"
    echo third >> \"$js\"
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CombineAppends));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo first >> \"$js\""]
        );
    }

    #[test]
    fn reports_unquoted_parameter_redirect_targets() {
        let source = "\
#!/bin/sh
for js in one; do
    echo first >> $js
    echo second >> $js
    echo third >> $js
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CombineAppends));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo first >> $js"]
        );
    }

    #[test]
    fn reports_append_runs_after_non_append_compound_statements() {
        let source = "\
#!/bin/sh
for js in one; do
    if [ -f $js ]; then
        sed -i '/marker/d' $js
    fi
    echo first >> $js
    echo second >> $js
    echo third >> $js
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CombineAppends));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo first >> $js"]
        );
    }

    #[test]
    fn counts_pipeline_echo_pipeline_runs() {
        let source = "\
#!/bin/sh
tr ' ' \"$nl\" < \"$tmpdepfile\" \\
  | sed -e 's/^.*\\.o://' -e 's/#.*$//' -e '/^$/ d' \\
  | tr \"$nl\" ' ' >> \"$depfile\"
echo >> \"$depfile\"
tr ' ' \"$nl\" < \"$tmpdepfile\" \\
  | sed -e 's/^.*\\.o://' -e 's/#.*$//' -e '/^$/ d' -e 's/$/:/' \\
  >> \"$depfile\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CombineAppends));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["tr \"$nl\" ' ' >> \"$depfile\""]
        );
    }

    #[test]
    fn keeps_trailing_arguments_after_redirect_targets_in_anchor_spans() {
        let source = "\
#!/bin/sh
echo >>$tools '. /tmp/loader'
echo >>$tools ''
echo >>$tools 'cat >script.sh <<EOF'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CombineAppends));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo >>$tools '. /tmp/loader'"]
        );
    }

    #[test]
    fn matches_gnunet_style_append_runs() {
        let source = "\
#!/bin/sh
for ffprofile in /home/$USER/.mozilla/firefox/*.*/; do
    js=$ffprofile/user.js
    if [ -f $js ]; then
        sed -i '/Preferences for using the GNU Name System/d' $js
        sed -i '/network.proxy.socks/d' $js
        sed -i '/network.proxy.socks_port/d' $js
        sed -i '/network.proxy.socks_remote_dns/d' $js
        sed -i '/network.proxy.type/d' $js
    fi
    echo \"// Preferences for using the GNU Name System\" >> $js
    echo \"user_pref(\\\"network.proxy.socks\\\", \\\"localhost\\\");\" >> $js
    echo \"user_pref(\\\"network.proxy.socks_port\\\", $PORT);\" >> $js
    echo \"user_pref(\\\"network.proxy.socks_remote_dns\\\", true);\" >> $js
    echo \"user_pref(\\\"network.proxy.type\\\", 1);\" >> $js
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CombineAppends));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo \"// Preferences for using the GNU Name System\" >> $js"]
        );
    }

    #[test]
    fn matches_depcomp_style_append_runs() {
        let source = "\
#!/bin/sh
tr ' ' \"$nl\" < \"$tmpdepfile\" \\
  | sed -e 's/^.*\\.o://' -e 's/#.*$//' -e '/^$/ d' \\
  | tr \"$nl\" ' ' >> \"$depfile\"
echo >> \"$depfile\"
# The second pass generates a dummy entry for each header file.
tr ' ' \"$nl\" < \"$tmpdepfile\" \\
  | sed -e 's/^.*\\.o://' -e 's/#.*$//' -e '/^$/ d' -e 's/$/:/' \\
  >> \"$depfile\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CombineAppends));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["tr \"$nl\" ' ' >> \"$depfile\""]
        );
    }

    #[test]
    fn anchors_pipeline_runs_at_the_redirecting_segment() {
        let source = "\
#!/bin/sh
printf '%s\\n' one \\
  | sed 's/o/o/' >> out.log
echo two >> out.log
echo three >> out.log
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CombineAppends));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["sed 's/o/o/' >> out.log"]
        );
    }

    #[test]
    fn ignores_top_level_append_runs_inside_case_arms() {
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

    #[test]
    fn reports_nested_append_runs_inside_case_arms() {
        let source = "\
#!/bin/sh
case \"$kind\" in
  kernel)
    if test -f \"$tmpdepfile\"; then
      tr ' ' \"$nl\" < \"$tmpdepfile\" \\
        | sed -e 's/^.*\\.o://' -e 's/#.*$//' -e '/^$/ d' \\
        | tr \"$nl\" ' ' >> \"$depfile\"
      echo >> \"$depfile\"
      tr ' ' \"$nl\" < \"$tmpdepfile\" \\
        | sed -e 's/^.*\\.o://' -e 's/#.*$//' -e '/^$/ d' -e 's/$/:/' \\
        >> \"$depfile\"
    fi
    ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CombineAppends));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["tr \"$nl\" ' ' >> \"$depfile\""]
        );
    }

    #[test]
    fn keeps_trailing_heredoc_redirects_in_anchor_spans() {
        let source = "\
#!/bin/sh
cat >>confdefs.h <<_ACEOF
#define PACKAGE_NAME \"$PACKAGE_NAME\"
_ACEOF
printf '%s\\n' done >>confdefs.h
printf '%s\\n' again >>confdefs.h
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CombineAppends));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["cat >>confdefs.h <<_ACEOF"]
        );
    }
}
