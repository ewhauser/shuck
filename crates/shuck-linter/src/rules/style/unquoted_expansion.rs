mod safe_value;

use rustc_hash::FxHashSet;
use shuck_ast::Span;

use self::safe_value::{S001QuoteExposure, SafeValueIndex, SafeValueQuery};
use crate::{
    Checker, ExpansionContext, FactSpan, Rule, ShellDialect, Violation, WordOccurrenceRef,
};

pub struct UnquotedExpansion;

impl Violation for UnquotedExpansion {
    fn rule() -> Rule {
        Rule::UnquotedExpansion
    }

    fn message(&self) -> String {
        "quote parameter expansions to avoid word splitting and globbing".to_owned()
    }
}

pub fn unquoted_expansion(checker: &mut Checker) {
    let source = checker.source();
    let colon_command_ids = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is(":"))
        .map(|fact| fact.id())
        .collect::<FxHashSet<_>>();
    let mut safe_values = SafeValueIndex::build(
        checker.semantic(),
        checker.semantic_analysis(),
        checker.facts(),
        source,
    );
    let array_assignment_split_spans = collect_array_assignment_split_candidate_spans(checker);
    let numeric_test_operand_spans = collect_numeric_test_operand_spans(checker, source);
    let plain_run_first_arg_spans = collect_plain_run_first_arg_spans(checker);
    let backtick_escaped_parameter_spans =
        checker.facts().backtick_escaped_parameter_reference_spans();

    let mut spans = Vec::new();
    let report_context = ReportContext {
        source,
        shell: checker.shell(),
        colon_command_ids: &colon_command_ids,
        numeric_test_operand_spans: &numeric_test_operand_spans,
        plain_run_first_arg_spans: &plain_run_first_arg_spans,
        backtick_escaped_parameter_spans,
    };
    for fact in checker.facts().word_facts() {
        collect_word_fact_reports(&report_context, &mut safe_values, &mut spans, fact);
        collect_array_assignment_split_reports(
            &mut safe_values,
            &mut spans,
            source,
            fact,
            &array_assignment_split_spans,
        );
    }
    let arithmetic_command_substitution_spans =
        checker.facts().arithmetic_command_substitution_spans();
    for fact in checker.facts().arithmetic_command_word_facts() {
        collect_arithmetic_word_fact_reports(
            &report_context,
            &mut safe_values,
            &mut spans,
            fact,
            arithmetic_command_substitution_spans,
        );
    }
    for escaped in checker.facts().backtick_escaped_parameters() {
        if escaped.standalone_command_name {
            continue;
        }
        if !escaped.name.as_ref().is_some_and(|name| {
            safe_values.name_reference_is_safe(name, escaped.reference_span, SafeValueQuery::Argv)
        }) {
            spans.push(escaped.diagnostic_span);
        }
    }
    for span in spans {
        checker.report_dedup(UnquotedExpansion, span);
    }
}

struct ReportContext<'a> {
    source: &'a str,
    shell: ShellDialect,
    colon_command_ids: &'a FxHashSet<crate::facts::core::CommandId>,
    numeric_test_operand_spans: &'a [Span],
    plain_run_first_arg_spans: &'a FxHashSet<FactSpan>,
    backtick_escaped_parameter_spans: &'a [Span],
}

fn collect_word_fact_reports(
    context: &ReportContext<'_>,
    safe_values: &mut SafeValueIndex<'_>,
    spans: &mut Vec<shuck_ast::Span>,
    fact: WordOccurrenceRef<'_, '_>,
) {
    let Some(expansion_context) = fact.host_expansion_context() else {
        return;
    };
    if !should_check_context(expansion_context, context.shell) {
        return;
    }
    report_word_expansions(
        spans,
        safe_values,
        context.source,
        fact,
        WordReportOptions {
            context: expansion_context,
            in_colon_command: context.colon_command_ids.contains(&fact.command_id()),
            numeric_test_operand_spans: context.numeric_test_operand_spans,
            plain_run_first_arg_spans: context.plain_run_first_arg_spans,
            backtick_escaped_parameter_spans: context.backtick_escaped_parameter_spans,
            part_filter: |_| true,
        },
    );
}

fn collect_arithmetic_word_fact_reports(
    context: &ReportContext<'_>,
    safe_values: &mut SafeValueIndex<'_>,
    spans: &mut Vec<shuck_ast::Span>,
    fact: WordOccurrenceRef<'_, '_>,
    arithmetic_command_substitution_spans: &[Span],
) {
    let Some(expansion_context) = fact.host_expansion_context() else {
        return;
    };
    if !should_check_context(expansion_context, context.shell) {
        return;
    }
    let fact_command_substitution_spans = fact.command_substitution_spans();
    let plain_run_first_arg_spans = FxHashSet::default();
    report_word_expansions(
        spans,
        safe_values,
        context.source,
        fact,
        WordReportOptions {
            context: expansion_context,
            in_colon_command: context.colon_command_ids.contains(&fact.command_id()),
            numeric_test_operand_spans: &[],
            plain_run_first_arg_spans: &plain_run_first_arg_spans,
            backtick_escaped_parameter_spans: context.backtick_escaped_parameter_spans,
            part_filter: |part_span| {
                arithmetic_word_follows_command_substitution(
                    part_span,
                    context.source,
                    fact_command_substitution_spans,
                ) || arithmetic_word_follows_command_substitution(
                    part_span,
                    context.source,
                    arithmetic_command_substitution_spans,
                )
            },
        },
    );
}

fn collect_numeric_test_operand_spans(checker: &Checker, source: &str) -> Vec<Span> {
    let mut spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.simple_test())
        .flat_map(|simple_test| {
            simple_test
                .numeric_binary_expression_operand_words(source)
                .into_iter()
                .map(|word| word.span)
        })
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup_by_key(|span| FactSpan::new(*span));
    spans
}

fn collect_plain_run_first_arg_spans(checker: &Checker) -> FxHashSet<FactSpan> {
    checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("run") && fact.wrappers().is_empty())
        .filter_map(|fact| {
            fact.body_args()
                .first()
                .map(|word| FactSpan::new(word.span))
        })
        .collect()
}

fn collect_array_assignment_split_candidate_spans(checker: &Checker) -> Vec<Span> {
    let mut spans = checker
        .facts()
        .array_assignment_split_word_facts()
        .flat_map(|fact| {
            let command_substitution_spans = fact.command_substitution_spans();
            fact.array_assignment_split_scalar_expansion_spans()
                .iter()
                .copied()
                .filter(move |span| {
                    command_substitution_spans
                        .iter()
                        .any(|outer| span_contains(*outer, *span))
                })
        })
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
    spans
}

fn collect_array_assignment_split_reports(
    safe_values: &mut SafeValueIndex<'_>,
    spans: &mut Vec<Span>,
    source: &str,
    fact: WordOccurrenceRef<'_, '_>,
    candidate_spans: &[Span],
) {
    if candidate_spans.is_empty() {
        return;
    }
    let Some(context) = fact.host_expansion_context() else {
        return;
    };
    if !array_assignment_split_context_is_checked(context) {
        return;
    }
    if context == ExpansionContext::CommandName
        && !fact.has_literal_affixes()
        && fact.parts_len() == 1
    {
        return;
    }

    for (part, part_span) in fact.parts_with_spans() {
        if !candidate_spans.contains(&part_span) {
            continue;
        }
        let affixed_command_name =
            context == ExpansionContext::CommandName && fact.has_literal_affixes();
        if safe_values.part_is_safe(part, part_span, SafeValueQuery::Argv) && !affixed_command_name
        {
            continue;
        }

        spans.push(fact.diagnostic_part_span(part, part_span, source));
    }
}

fn array_assignment_split_context_is_checked(context: ExpansionContext) -> bool {
    matches!(
        context,
        ExpansionContext::CommandName
            | ExpansionContext::CommandArgument
            | ExpansionContext::HereString
            | ExpansionContext::RedirectTarget(_)
            | ExpansionContext::DescriptorDupTarget(_)
    )
}

fn should_check_context(context: ExpansionContext, shell: ShellDialect) -> bool {
    match context {
        ExpansionContext::CommandName
        | ExpansionContext::CommandArgument
        | ExpansionContext::HereString
        | ExpansionContext::RedirectTarget(_)
        | ExpansionContext::DescriptorDupTarget(_) => true,
        ExpansionContext::DeclarationAssignmentValue => shell != ShellDialect::Bash,
        _ => false,
    }
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

fn arithmetic_word_follows_command_substitution(
    word_span: Span,
    source: &str,
    command_substitution_spans: &[Span],
) -> bool {
    command_substitution_spans.iter().copied().any(|span| {
        if span.end.offset > word_span.start.offset {
            return false;
        }
        let Some(between) = source.get(span.end.offset..word_span.start.offset) else {
            return false;
        };
        !between.contains('\n') && between.chars().all(char::is_whitespace)
    })
}

struct WordReportOptions<'a, F> {
    context: ExpansionContext,
    in_colon_command: bool,
    numeric_test_operand_spans: &'a [Span],
    plain_run_first_arg_spans: &'a FxHashSet<FactSpan>,
    backtick_escaped_parameter_spans: &'a [Span],
    part_filter: F,
}

fn report_word_expansions<F>(
    spans: &mut Vec<Span>,
    safe_values: &mut SafeValueIndex<'_>,
    source: &str,
    fact: WordOccurrenceRef<'_, '_>,
    options: WordReportOptions<'_, F>,
) where
    F: Fn(Span) -> bool,
{
    let WordReportOptions {
        context,
        in_colon_command,
        numeric_test_operand_spans,
        plain_run_first_arg_spans,
        backtick_escaped_parameter_spans,
        part_filter,
    } = options;

    if !fact.analysis().hazards.field_splitting && !fact.analysis().hazards.pathname_matching {
        return;
    }

    let scalar_spans = fact.scalar_expansion_spans();
    let assign_default_spans = if in_colon_command && context == ExpansionContext::CommandArgument {
        fact.unquoted_assign_default_spans()
    } else {
        Default::default()
    };
    let use_replacement_spans = fact.use_replacement_spans();
    let star_spans = fact.unquoted_star_parameter_spans();
    if scalar_spans.is_empty() && star_spans.is_empty() {
        return;
    }
    if context == ExpansionContext::CommandName
        && !fact.has_literal_affixes()
        && fact.parts_len() == 1
    {
        return;
    }
    if context == ExpansionContext::CommandArgument
        && plain_run_first_arg_spans.contains(&FactSpan::new(fact.span()))
    {
        return;
    }
    let Some(query) = SafeValueQuery::from_context(context) else {
        return;
    };
    for (part, part_span) in fact.parts_with_spans() {
        let report_unquoted_star = star_spans.contains(&part_span);
        if !scalar_spans.contains(&part_span) && !report_unquoted_star {
            continue;
        }
        if !part_filter(part_span) {
            continue;
        }
        if assign_default_spans.contains(&part_span) {
            continue;
        }
        if use_replacement_spans.contains(&part_span) {
            continue;
        }
        if fact.part_is_inside_backtick_escaped_double_quotes(part_span, source) {
            continue;
        }
        if backtick_escaped_parameter_spans
            .iter()
            .copied()
            .any(|span| span_contains(span, part_span))
        {
            continue;
        }
        if safe_values
            .part_is_safe_initializer_command_substitution_self_reference(part, part_span, query)
        {
            continue;
        }
        if safe_values.part_is_safe_initializer_command_substitution_static_setup_reference(
            part, part_span, query,
        ) {
            continue;
        }
        let in_numeric_test_operand =
            part_is_in_numeric_test_operand(part_span, numeric_test_operand_spans);
        if in_numeric_test_operand {
            if safe_values.part_is_safe(part, part_span, SafeValueQuery::NumericTestOperand) {
                continue;
            }
            if safe_values.part_has_s001_arithmetic_numeric_operand_exposure(part, part_span) {
                continue;
            }
        } else if context == ExpansionContext::CommandArgument
            && !fact.has_literal_affixes()
            && fact.parts_len() == 1
            && safe_values.part_has_s001_standalone_numeric_argv_exposure(part, part_span)
        {
            continue;
        }
        let exposure = safe_values.part_s001_quote_exposure(part, part_span, query);
        if matches!(exposure, S001QuoteExposure::QuoteInertNonEmpty) {
            continue;
        }
        if !safe_values.span_has_s001_function_unset_exposure(part_span, query)
            && safe_values.part_is_safe(part, part_span, query)
        {
            continue;
        }

        spans.push(fact.diagnostic_part_span(part, part_span, source));
    }
}

fn part_is_in_numeric_test_operand(part_span: Span, operand_spans: &[Span]) -> bool {
    operand_spans
        .iter()
        .copied()
        .any(|operand_span| span_contains(operand_span, part_span))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_snippet, test_snippet_at_path};
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_scalar_expansion_parts_instead_of_whole_words() {
        let source = "\
#!/bin/bash
printf '%s\\n' prefix${name}suffix ${arr[0]} ${arr[@]}
printf '%s\\n' ${arr[@]:-fallback} ${arr[*]:-fallback} ${arr[@]@Q} ${arr[*]@Q} ${arr[0]:-fallback}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${name}",
                "${arr[0]}",
                "${arr[*]:-fallback}",
                "${arr[*]@Q}",
                "${arr[0]:-fallback}"
            ]
        );
    }

    #[test]
    fn descends_into_nested_command_substitutions() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"$(echo $name)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$name"]
        );
    }

    #[test]
    fn descends_into_arithmetic_command_substitutions() {
        let source = "\
#!/bin/bash
if (( $(du -c $profraw_file_mask | tail -n 1 | cut -f 1) == 0 )); then
  :
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$profraw_file_mask"]
        );
    }

    #[test]
    fn reports_arithmetic_words_following_command_substitutions() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"$(( $(cat \"$backlight/brightness\") $1 step ))\"
printf '%s\\n' \"$(( $(cat \"$backlight/brightness\") + $count ))\"
printf '%s\\n' \"$(( $count + 1 ))\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$1"]
        );
    }

    #[test]
    fn ignores_backtick_escaped_double_quoted_parameters() {
        let source = "\
#!/bin/bash
path=\"`dirname \\\"$REALPATH\\\"`\"
flags=\"`echo \\\"$CFLAGS\\\" | sed \\\"s/ $COVERAGE_FLAGS//\\\"`\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn treats_brace_fd_redirect_bindings_as_numeric_values() {
        let source = "\
#!/bin/bash
exec {fd}< <(:)
read -u $fd value
echo >&$fd
echo 2>&$fd
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_descriptor_dup_target_before_new_brace_fd_binding() {
        let source = "\
#!/bin/bash
fd='1 2'
exec {fd}>&$fd
printf '%s\\n' \"$fd\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$fd"]
        );
    }

    #[test]
    fn reports_descriptor_dup_targets_without_visible_fd_bindings() {
        let source = "\
#!/bin/bash
function start() {
  exec {_GITSTATUS_REQ_FD}>>\"$req_fifo\" {_GITSTATUS_RESP_FD}<\"$resp_fifo\" || return
  IFS='' read -r -u $_GITSTATUS_RESP_FD GITSTATUS_DAEMON_PID || return
}
function query() {
  echo -nE \"$req_id\"$'\\x1f'\"$dir\"$'\\x1e' >&$_GITSTATUS_REQ_FD || return
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$_GITSTATUS_REQ_FD"]
        );
    }

    #[test]
    fn reports_literal_bindings_after_exit_like_function_calls() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { exit 0; }
Exit
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${OPTION_BINARY_FILE}", "${OPENBSD_CONTENTS}"]
        );
    }

    #[test]
    fn reports_literal_bindings_after_early_exit_like_function_calls() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { exit 0; :; }
Exit
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${OPTION_BINARY_FILE}", "${OPENBSD_CONTENTS}"]
        );
    }

    #[test]
    fn reports_literal_bindings_after_assigned_exit_like_function_calls() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { FOO=1 exit 0; }
Exit
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${OPTION_BINARY_FILE}", "${OPENBSD_CONTENTS}"]
        );
    }

    #[test]
    fn reports_literal_bindings_after_negated_exit_like_function_calls() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { ! exit 0; }
Exit
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${OPTION_BINARY_FILE}", "${OPENBSD_CONTENTS}"]
        );
    }

    #[test]
    fn reports_literal_bindings_after_extra_arg_exit_like_function_calls() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { exit 0 1; }
Exit
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${OPTION_BINARY_FILE}", "${OPENBSD_CONTENTS}"]
        );
    }

    #[test]
    fn ignores_pre_definition_exit_like_calls_before_function_definitions() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit
Exit() { exit 0; }
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_returning_helpers_with_unreachable_trailing_exit() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { return 0; exit 1; }
Exit
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_safe_bindings_after_conditionally_returning_exit_like_helpers() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { if [ \"$SKIP\" ]; then return 0; fi; exit 1; }
Exit
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_safe_bindings_after_all_branch_returning_helpers() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() {
  if [ \"$SKIP\" ]; then
    return 0
  else
    return 1
  fi
  exit 0
}
Exit
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_literal_bindings_after_conditionally_exiting_exit_like_helpers() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { if [ \"$SKIP\" ]; then exit 1; fi; exit 0; }
Exit
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${OPTION_BINARY_FILE}", "${OPENBSD_CONTENTS}"]
        );
    }

    #[test]
    fn ignores_safe_bindings_after_conditional_exit_like_helper_calls() {
        let source = "\
#!/bin/sh
LIBDIRSUFFIX=64
warn_accounts() { exit 1; }
if false; then
  warn_accounts
fi
echo /usr/lib${LIBDIRSUFFIX}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_literal_bindings_after_redirected_exit_like_function_calls() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { exit 0; }
Exit >/dev/null
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${OPTION_BINARY_FILE}", "${OPENBSD_CONTENTS}"]
        );
    }

    #[test]
    fn ignores_safe_bindings_after_background_exit_like_helper_calls() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { exit 0; }
Exit &
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_safe_bindings_after_backgrounded_brace_group_exit_like_calls() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { exit 0; }
{ Exit; } &
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_safe_bindings_after_shadowed_exit_like_helper_names() {
        let source = "\
#!/bin/sh
Exit() { exit 0; }
Exit() { :; }
SAFE=foo
Exit
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_safe_bindings_after_conditionally_defined_exit_like_helpers() {
        let source = "\
#!/bin/sh
SAFE=foo
if false; then
  Exit() { exit 0; }
fi
Exit
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_safe_bindings_after_backgrounded_exit_like_helper_definitions() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { exit 0; } &
Exit
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn collapses_multiline_backtick_spans_to_shellcheck_columns() {
        let source = "\
#!/bin/sh
mkdir_umask=`expr $umask + 22 \\
  - $umask % 100 % 40 + $umask % 20 \\
  - $umask % 10 % 4 + $umask % 2
`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (2, 19, 2, 25),
                (2, 35, 2, 41),
                (2, 55, 2, 61),
                (2, 71, 2, 77),
                (2, 89, 2, 95),
            ]
        );
    }

    #[test]
    fn collapses_tab_indented_multiline_backtick_spans_to_shellcheck_columns() {
        let source = "\
#!/bin/sh
\t    mkdir_umask=`expr $umask + 22 \\
\t      - $umask % 100 % 40 + $umask % 20 \\
\t      - $umask % 10 % 4 + $umask % 2
\t    `
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (2, 24, 2, 30),
                (2, 50, 2, 56),
                (2, 70, 2, 76),
                (2, 98, 2, 104),
                (2, 116, 2, 122),
            ]
        );
    }

    #[test]
    fn collapses_crlf_multiline_backtick_spans_to_shellcheck_columns() {
        let source = "#!/bin/sh\r\nmkdir_umask=`expr $umask + 22 \\\r\n  - $umask % 100 % 40 + $umask % 20 \\\r\n  - $umask % 10 % 4 + $umask % 2\r\n`\r\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (2, 19, 2, 25),
                (2, 35, 2, 41),
                (2, 55, 2, 61),
                (2, 71, 2, 77),
                (2, 89, 2, 95),
            ]
        );
    }

    #[test]
    fn reports_unquoted_expansions_after_brace_group_wrapped_exit_like_calls() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { exit 0; }
{ Exit; }
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$SAFE"]
        );
    }

    #[test]
    fn ignores_exit_like_helper_calls_in_uncalled_function_bodies() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { exit 0; }
wrapper() {
  Exit
}
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_unquoted_expansions_after_exit_like_calls_inside_same_function_body() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { exit 0; }
wrapper() {
  Exit
  echo /tmp/$SAFE
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$SAFE"]
        );
    }

    #[test]
    fn reports_unquoted_expansions_after_inline_returns_in_same_function_body() {
        let source = "\
#!/bin/sh
wrapper() {
  SAFE=foo
  return 0
  echo /tmp/$SAFE
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$SAFE"]
        );
    }

    #[test]
    fn reports_unquoted_expansions_after_all_branch_returns() {
        let source = "\
#!/bin/bash
wrapper() {
  local good=0
  if cond; then
    return $good
  else
    return $good
  fi
  return $good
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$good"]
        );
    }

    #[test]
    fn reports_unquoted_expansions_after_deferred_exit_like_calls_resolved_by_later_helpers() {
        let source = "\
#!/bin/sh
SAFE=foo
wrapper() {
  Exit
  echo /tmp/$SAFE
}
Exit() { exit 0; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$SAFE"]
        );
    }

    #[test]
    fn ignores_expansions_inside_quoted_fragments_of_mixed_words() {
        let source = "\
#!/bin/bash
exec dbus-send --bus=\"unix:path=$XDG_RUNTIME_DIR/bus\" / org.freedesktop.DBus.Peer.Ping
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_only_unquoted_fragments_of_mixed_words() {
        let source = "\
#!/bin/bash
printf '%s\\n' prefix\"$HOME\"/$suffix
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$suffix"]
        );
    }

    #[test]
    fn reports_unquoted_expansions_in_case_cli_dispatch_entry_functions_with_top_level_exit_status()
    {
        let source = "\
#!/bin/sh
exec=/usr/sbin/collectd
pidfile=/var/run/collectd.pid
configfile=/etc/collectd.conf

start() {
  [ -x $exec ] || exit 5
  [ -f $pidfile ] && rm $pidfile
  $exec -P $pidfile -C $configfile
}

case \"$1\" in
  start) $1 ;;
esac
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$exec", "$pidfile", "$pidfile", "$pidfile", "$configfile"]
        );
    }

    #[test]
    fn reports_collectd_style_case_cli_dispatch_entry_functions() {
        let source = "\
#!/bin/sh
exec=/usr/sbin/collectd
prog=$(basename $exec)
configfile=/etc/collectd.conf
pidfile=/var/run/collectd.pid

start() {
  [ -x $exec ] || exit 5
  if [ -f $pidfile ]; then
    echo \"Seems that an active process is up and running with pid $(cat $pidfile)\"
    echo \"If this is not true try first to remove pidfile $pidfile\"
    exit 5
  fi
  echo $\"Starting $prog\"
  $exec -P $pidfile -C $configfile
}

stop() {
  if [ -e $pidfile ]; then
    echo \"Stopping $prog\"
    kill -QUIT $(cat $pidfile) 2>/dev/null
    rm $pidfile
  fi
}

status() {
  echo -n \"$prog is \"
  CHECK=$(ps aux | grep $exec | grep -v grep)
  STATUS=$?
  if [ \"$STATUS\" == \"1\" ]; then
    echo \"not running\"
  else
    echo \"running\"
  fi
}

restart() {
  stop
  start
}

case \"$1\" in
  start)
    $1
    ;;
  stop)
    $1
    ;;
  restart)
    $1
    ;;
  status)
    $1
    ;;
esac
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$exec",
                "$pidfile",
                "$pidfile",
                "$pidfile",
                "$configfile",
                "$pidfile",
                "$pidfile",
                "$pidfile",
                "$exec",
            ]
        );
    }

    #[test]
    fn reports_spice_vdagent_style_case_cli_dispatch_entry_functions() {
        let source = "\
#!/bin/sh
exec=\"/usr/sbin/spice-vdagentd\"
prog=\"spice-vdagentd\"
port=\"/dev/virtio-ports/com.redhat.spice.0\"
pid=\"/var/run/spice-vdagentd/spice-vdagentd.pid\"
lockfile=/var/lock/subsys/$prog

start() {
  /sbin/modprobe uinput > /dev/null 2>&1
  /usr/bin/rm -f /var/run/spice-vdagentd/spice-vdagent-sock
  /usr/bin/mkdir -p /var/run/spice-vdagentd
  /usr/bin/echo \"Starting $prog: \"
  $exec -s $port
  retval=$?
  /usr/bin/echo
  [ $retval -eq 0 ] && echo \"$(pidof $prog)\" > $pid && /usr/bin/touch $lockfile
  return $retval
}

stop() {
  if [ \"$(pidof $prog)\" ]; then
    /usr/bin/echo \"Stopping $prog: \"
    /bin/kill $(cat $pid)
  else
    /usr/bin/echo \"$prog not running\"
    return 1
  fi
  retval=$?
  /usr/bin/echo
  [ $retval -eq 0 ] && rm -f $lockfile $pid
  return $retval
}

restart() {
  stop
  start
}

case \"$1\" in
  start)
    $1
    ;;
  stop)
    $1
    ;;
  restart)
    $1
    ;;
  *)
    /usr/bin/echo $\"Usage: $0 {start|stop|restart}\"
    exit 2
esac
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));
        let spans = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans.iter().filter(|span| **span == "$port").count(), 1);
        assert_eq!(spans.iter().filter(|span| **span == "$retval").count(), 4);
        assert_eq!(spans.iter().filter(|span| **span == "$prog").count(), 2);
        assert_eq!(spans.iter().filter(|span| **span == "$pid").count(), 3);
        assert_eq!(spans.iter().filter(|span| **span == "$lockfile").count(), 2);
    }

    #[test]
    fn reports_case_cli_dispatch_entry_function_local_arguments() {
        let source = "\
#!/bin/sh
start() {
  local n=\"$name\"
  echo $n
}

case \"$1\" in
  start) $1 ;;
esac
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$n"]
        );
    }

    #[test]
    fn reports_case_cli_dispatch_entry_function_local_arguments_with_literal_exit() {
        let source = "\
#!/bin/sh
start() {
  local n=\"$name\"
  echo $n
}

case \"$1\" in
  start) $1 ;;
esac
exit 0
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$n"]
        );
    }

    #[test]
    fn reports_case_cli_reachable_helper_local_arguments() {
        let source = "\
#!/bin/sh
foo() {
  local n=\"$name\"
  echo $n
}

bound() { foo; }
renew() { foo; }
deconfig() { :; }

case \"$1\" in
  deconfig|renew|bound) $1 ;;
esac
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$n"]
        );
    }

    #[test]
    fn reports_case_cli_pre_dispatch_function_local_arguments() {
        let source = "\
#!/bin/sh
foo() {
  local n=\"$name\"
  echo $n
}

bar() { :; }

case \"$1\" in
  bar) $1 ;;
esac
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$n"]
        );
    }

    #[test]
    fn does_not_broaden_dynamic_case_dispatch_without_top_level_exit_status() {
        let source = "\
#!/bin/sh
exec=/usr/sbin/collectd
pidfile=/var/run/collectd.pid

start() {
  [ -x $exec ] || exit 5
  $exec -P $pidfile
}

case \"$1\" in
  start) $1 ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_function_local_arguments_before_plain_top_level_exit() {
        let source = "\
#!/bin/sh
start() {
  local n=\"$name\"
  echo $n
}
exit 0
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$n"]
        );
    }

    #[test]
    fn reports_nested_function_return_arguments_without_top_level_exit() {
        let source = "\
#!/bin/bash
outer() {
  inner() {
    local good=0
    return $good
  }
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$good"]
        );
    }

    #[test]
    fn keeps_unknown_option_bundle_warnings_without_broad_command_name_reports() {
        let source = "\
#!/bin/sh
BLUEALSA_BIN=/usr/bin/bluealsa

start() {
  $BLUEALSA_BIN $BLUEALSA_OPTS
}

case \"$1\" in
  start) $1 ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$BLUEALSA_OPTS"]
        );
    }

    #[test]
    fn ignores_static_case_dispatch_calls() {
        let source = "\
#!/bin/sh
exec=/usr/sbin/collectd

start() {
  $exec
}

case \"$1\" in
  start) start ;;
esac
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn skips_for_lists_but_reports_here_strings_and_redirect_targets() {
        let source = "\
#!/bin/bash
for item in $first \"$second\"; do :; done
cat <<< $here >$out
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$here", "$out"]
        );
    }

    #[test]
    fn skips_assignment_values_and_reports_descriptor_dup_targets() {
        let source = "\
#!/bin/bash
value=$name
printf '%s\\n' ok >&$fd
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$fd"]
        );
    }

    #[test]
    fn reports_unquoted_zsh_parameter_modifiers() {
        let source = "\
#!/usr/bin/env zsh
print ${~foo}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${~foo}"]
        );
    }

    #[test]
    fn reports_dynamic_command_names() {
        let source = "\
#!/bin/bash
$HOME/bin/tool $arg
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$HOME", "$arg"]
        );
    }

    #[test]
    fn reports_unquoted_star_selector_expansions() {
        let source = "\
#!/bin/bash
RSYNC_OPTIONS=(-a -v)
rsync ${RSYNC_OPTIONS[*]} src dst
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${RSYNC_OPTIONS[*]}"]
        );
    }

    #[test]
    fn reports_bourne_transformations_in_command_arguments() {
        let source = "\
#!/bin/bash
printf '%s\\n' ${name@U}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${name@U}"]
        );
    }

    #[test]
    fn reports_bindings_derived_from_parameter_operations() {
        let source = "\
#!/bin/bash
PRGNAM=Fennel
SRCNAM=${PRGNAM,}
release=1.0.0
VERSION=${release:-fallback}
rm -rf $SRCNAM-$VERSION
printf '%s\\n' ${PRGNAM,} ${release:-fallback}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$SRCNAM", "$VERSION"]
        );
    }

    #[test]
    fn reports_bindings_from_short_circuit_assignment_ternaries() {
        let source = "\
#!/bin/bash
check() { return 0; }
check && w='-w' || w=''
if check; then
  flag='-w'
else
  flag=''
fi
iptables $w -t nat -N chain
iptables $flag -t nat -N chain
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$w"]
        );
    }

    #[test]
    fn skips_numeric_short_circuit_assignment_ternaries() {
        let source = "\
#!/bin/bash
I=1
while [ $I -le 3 ]; do
  [[ -z $SPEED ]] && I=$(( I + 1 )) || I=11
done
J=1
while [ $J -le 3 ]; do
  [[ -z $SPEED ]] && J=+11 || J=-1
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_numeric_short_circuit_values_outside_numeric_tests() {
        let source = "\
#!/bin/bash
f() {
  RV=0
  [[ -z $PID ]] && RV=0 || RV=1
  return $RV
}
[ \"${WITH_POWER_PLANS}\" != \"no\" ] && __INCLUDE_POWER_PLANS=1 || __INCLUDE_POWER_PLANS=0
make install INCLUDE_POWER_PLANS=${__INCLUDE_POWER_PLANS}
[ \"${JBIG}\" = \"no\" ] && JBIGOPT=0 || JBIGOPT=1
make DISABLE_JBIG=$JBIGOPT
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$RV", "${__INCLUDE_POWER_PLANS}", "$JBIGOPT"]
        );
    }

    #[test]
    fn reports_nested_guarded_short_circuit_assignment_ternaries() {
        let source = "\
#!/bin/bash
f() {
  [ \"$1\" = iptables ] && {
    true && w='-w' || w=''
  }
  [ \"$1\" = ip6tables ] && {
    true && w='-w' || w=''
  }
  iptables $w -t nat -N chain
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$w"]
        );
    }

    #[test]
    fn skips_colon_assign_default_expansions_but_keeps_regular_argument_cases() {
        let source = "\
#!/bin/bash
: ${x:=fallback} $other
printf '%s\\n' ${z:=fallback}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.slice(source)))
                .collect::<Vec<_>>(),
            vec![(2, "$other"), (3, "${z:=fallback}")]
        );
    }

    #[test]
    fn keeps_colon_assign_default_reports_for_here_strings_and_redirect_targets() {
        let source = "\
#!/bin/bash
: <<< ${x:=fallback} >${y:=out}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${x:=fallback}", "${y:=out}"]
        );
    }

    #[test]
    fn reports_dynamic_values_inside_nested_command_substitution_arguments() {
        let source = "\
#!/bin/sh
PRGNAM=cproc
GIT_SHA=$( git rev-parse --short HEAD )
DATE=$( git log --date=format:%Y%m%d --format=%cd | head -1 )
VERSION=${DATE}_${GIT_SHA}
echo \"MD5SUM=\\\"$( md5sum $PRGNAM-$VERSION.tar.xz | cut -d' ' -f1 )\\\"\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$VERSION"]
        );
    }

    #[test]
    fn skips_self_references_inside_command_substitution_initializers() {
        let source = "\
#!/bin/sh
check=0
check=$(expr $check + $?)
value=$1
value=$(echo $value | sed 's/^ //')
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$value"]
        );
    }

    #[test]
    fn skips_static_setup_references_inside_command_substitution_initializers() {
        let source = "\
#!/bin/bash
scope_menu() {
  case $scope in
    global) WAFSCOPE=CLOUDFRONT ;;
    regional) WAFSCOPE=REGIONAL ;;
    *) exit 1 ;;
  esac
}
get_set() {
  result=$(aws waf get --scope $WAFSCOPE --profile $profile 2>&1)
}
update_set() {
  get_set
}
unused_export() {
  get_set
}
main() {
  update_set
}
scope_menu
main
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$profile"]
        );
    }

    #[test]
    fn reports_static_setup_references_without_path_coverage() {
        let source = "\
#!/bin/bash
if [ \"$scope\" = global ]; then
  WAFSCOPE=CLOUDFRONT
fi
result=$(aws waf get --scope $WAFSCOPE)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$WAFSCOPE"]
        );
    }

    #[test]
    fn reports_dynamic_values_inside_parameter_replacement_command_substitutions() {
        let source = "\
#!/bin/bash
image_file='foo bar'
template='IMG_EXTRACT_SIZE IMG_SHA256 IMG_DOWNLOAD_SIZE'
template=\"${template/IMG_EXTRACT_SIZE/$(stat -c %s $image_file)}\"
template=\"${template/IMG_SHA256/$(sha256sum $image_file | cut -d' ' -f1)}\"
template=\"${template/IMG_DOWNLOAD_SIZE/$(stat -c %s ${image_file}.xz)}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$image_file", "$image_file", "${image_file}"]
        );
    }

    #[test]
    fn reports_dynamic_values_inside_parameter_replacement_arithmetic_expansions() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"${template/IMG_OFFSET/$(( $(cat file) $1 step ))}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$1"]
        );
    }

    #[test]
    fn reports_dynamic_values_inside_parameter_default_arithmetic_expansions() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"${value:-$(( $(cat file) $1 step ))}\" \"${value:=$(( $2 + 1 ))}\" \"${value:+$(( $3 + 1 ))}\" \"${value:?$(( $4 + 1 ))}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$1"]
        );
    }

    #[test]
    fn reports_dynamic_values_inside_arithmetic_shell_words() {
        let source = "\
#!/bin/sh
printf '%s' \"$(( $(cat file) $1 step ))\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$1"]
        );
    }

    #[test]
    fn reports_scalar_expansions_that_split_through_array_assignments() {
        let source = "\
#!/bin/bash
MODE_ID+=($group-$id)
MODE_CUR=($(get_${HAS_MODESET}_mode_info \"${mode_id[*]}\"))
arr=(\"$(printf '%s\\n' $quoted_outer)\")
arr=($PPID $HOME)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${HAS_MODESET}", "$quoted_outer"]
        );
    }

    #[test]
    fn reports_possibly_uninitialized_static_branch_bindings_in_array_assignments() {
        let source = "\
#!/bin/bash
if command -v kms >/dev/null; then
  HAS_MODESET=kms
elif command -v xrandr >/dev/null; then
  HAS_MODESET=x11
fi

mode_switch() {
  if [ \"$HAS_MODESET\" = kms ]; then
    MODE_CUR=($(get_${HAS_MODESET}_mode_info))
  fi
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${HAS_MODESET}"]
        );
    }

    #[test]
    fn skips_standalone_safe_command_names_in_array_assignment_substitutions() {
        let source = "\
#!/bin/bash
tool=/usr/bin/helper
items=($($tool list))
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_standalone_dynamic_command_names_in_array_assignment_substitutions() {
        let source = "\
#!/bin/bash
items=($($tool list))
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_for_lists_in_array_assignment_substitutions() {
        let source = "\
#!/bin/bash
items=($(
  for keyword in $keywords; do
    printf '%s\\n' \"$keyword\"
  done
))
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn treats_wrapper_targets_as_command_names() {
        let source = "\
#!/bin/sh
exec $SHELL $0 \"$@\"
exec pre$x arg
command $tool arg
builtin $name arg
case \"$1\" in
  restart) $0 stop; exec $0 start ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$0", "$x"]
        );
    }

    #[test]
    fn skips_safe_variables_that_share_names_with_functions() {
        let source = "\
#!/usr/bin/env bash
check_gitlab=0
check_gitlab() {
  [ $check_gitlab = 1 ] && return 0
}
if check_gitlab; then
  :
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_escaped_parameters_in_legacy_backticks() {
        let source = "\
#!/bin/sh
SAFE=foo
printf '%s\\n' `echo \\$1 \\$HOME \\$SAFE \\$PPID`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![(3, 21, 3, 23), (3, 24, 3, 29)]
        );
    }

    #[test]
    fn ignores_escaped_parameters_in_quoted_heredoc_backticks() {
        let source = "\
#!/bin/sh
cat <<'EOF'
`echo \\$HOME`
EOF
cat <<EOF
`echo \\$HOME`
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![(6, 7, 6, 12)]
        );
    }

    #[test]
    fn reports_unsafe_escaped_parameters_in_legacy_backticks() {
        let source = "\
#!/bin/sh
if cond; then
  value=ok
fi
printf '%s\\n' `echo \\$value`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![(5, 21, 5, 27)]
        );
    }

    #[test]
    fn reports_complex_escaped_parameters_in_legacy_backticks() {
        let source = "\
#!/bin/sh
printf '%s\\n' `echo \\${SAFE:-$fallback} \\${SAFE:+$fallback}`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![(2, 21, 2, 39)]
        );
    }

    #[test]
    fn skips_escaped_backtick_standalone_command_names() {
        let source = "\
#!/bin/sh
`\\$cmd`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_escaped_backtick_command_names_after_quoted_assignment_prefixes() {
        let source = "\
#!/bin/sh
`VAR=\"a b\" OTHER=$(printf '%s\\n' value) \\$cmd arg`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_escaped_backtick_command_names_after_append_assignment_prefixes() {
        let source = "\
#!/bin/sh
`VAR+=x \\$cmd arg`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_escaped_backtick_command_names_after_redirection_prefixes() {
        let source = "\
#!/bin/sh
`>/tmp/out \\$cmd arg`
`2>/tmp/err FOO=bar \\$other arg`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_escaped_backtick_command_arguments() {
        let source = "\
#!/bin/sh
`echo \\$arg`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn reports_affixed_escaped_backtick_command_names() {
        let source = "\
#!/bin/sh
`pre\\$cmd`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn skips_use_replacement_expansions() {
        let source = "\
#!/bin/bash
foo='a b'
arr=('left side' right)
printf '%s\\n' ${foo:+$foo} ${foo:+\"$foo\"} ${arr:+\"${arr[@]}\"}
tar ${foo:+-C \"$foo\"} -f archive.tar
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn keeps_default_expansions_with_quoted_operands() {
        let source = "\
#!/bin/bash
foo='a b'
printf '%s\\n' ${foo:-\"$foo\"}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${foo:-\"$foo\"}"]
        );
    }

    #[test]
    fn skips_plain_expansion_command_names_but_reports_composite_command_words() {
        let source = "\
#!/bin/bash
$CC -c file.c
if $TERMUX_ON_DEVICE_BUILD; then
  :
fi
${CC}${FLAGS} file.c
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CC}", "${FLAGS}"]
        );
    }

    #[test]
    fn ignores_escaped_backticks_inside_double_quoted_assignments() {
        let source = "\
#!/bin/bash
NVM_TEST_VERSION=v0.42
EXPECTED=\"Found '$(pwd)/.nvmrc' with version <${NVM_TEST_VERSION}>
N/A: version \\\"${NVM_TEST_VERSION}\\\" is not yet installed.

You need to run \\`nvm install ${NVM_TEST_VERSION}\\` to install and use it.
No NODE_VERSION provided; no .nvmrc file found\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn reports_expansions_wrapped_in_escaped_literal_quotes() {
        let source = "\
#!/bin/bash
printf '%s\\n' -DPACKAGE_VERSION=\\\"$TERMUX_PKG_VERSION\\\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$TERMUX_PKG_VERSION"]
        );
    }

    #[test]
    fn anchors_braced_parameters_inside_escaped_literal_quotes() {
        let source = "\
#!/bin/bash
printf '%s\\n' \\\"${items[*]}\\\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${items[*]}"]
        );
    }

    #[test]
    fn anchors_braced_parameters_inside_escaped_literal_quotes_in_substitution() {
        let source = "\
#!/bin/bash
json=\"$(echo \\\"${items[*]}\\\")\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${items[*]}"]
        );
    }

    #[test]
    fn reports_inner_parameter_inside_escaped_indirect_template() {
        let source = "\
#!/bin/sh
tool=pack
archive=out
$tool archive \"${archive}\" \"Test $1\" echo \\\"\\${${1}}\\\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${1}"]
        );
    }

    #[test]
    fn reports_nested_substitution_arguments_after_escaped_quote_default_segments() {
        let source = "\
#!/bin/sh
label=\",label=\\\"${fallback:=value}$(render value $line)\\\"\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$line"]
        );
    }

    #[test]
    fn anchors_nested_substitution_arguments_to_physical_lines_after_escaped_continuations() {
        let source = "\
#!/bin/bash
echo \"script
  LEFT=\"$left\":\\$base \\
    CHILD=\\$base/$(basename $child) \\
    PATH=$path
    run\" > out
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (
                    diagnostic.span.start.line,
                    diagnostic.span.start.column,
                    diagnostic.span.end.line,
                    diagnostic.span.end.column,
                    diagnostic.span.slice(source),
                ))
                .collect::<Vec<_>>(),
            vec![(3, 9, 3, 14, "$left"), (4, 29, 4, 35, "$child")]
        );
    }

    #[test]
    fn reports_decl_assignment_values_in_sh_mode() {
        let source = "\
local _patch=$TERMUX_PKG_BUILDER_DIR/file.diff
export PATH=$HOME/bin:$PATH
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/pkg.sh"),
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$TERMUX_PKG_BUILDER_DIR", "$HOME", "$PATH"]
        );
    }

    #[test]
    fn reports_transformed_decl_assignment_values_in_sh_mode() {
        let source = "\
local upper=${TERMUX_ARCH@U}
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/pkg.sh"),
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${TERMUX_ARCH@U}"]
        );
    }

    #[test]
    fn skips_decl_assignment_values_in_bash_mode() {
        let source = "\
#!/bin/bash
local _patch=$TERMUX_PKG_BUILDER_DIR/file.diff
export PATH=$HOME/bin:$PATH
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/pkg.bash"),
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_unquoted_spans_inside_mixed_words() {
        let source = "\
#!/bin/bash
printf '%s\\n' 'prefix:'$name':suffix'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$name"]
        );
    }

    #[test]
    fn skips_safe_special_parameters() {
        let source = "\
#!/bin/bash
printf '%s\\n' $? $# $$ $! $- $0 $1 $* $@
run || return $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$0", "$1", "$*"]
        );
    }

    #[test]
    fn skips_bindings_with_safe_visible_values() {
        let source = "\
#!/bin/bash
n=42
s=abc
glob='*'
split='1 2'
copy=\"$n\"
alias=$s
printf '%s\\n' $n $s $glob $split $copy $alias
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$glob", "$split"]
        );
    }

    #[test]
    fn reports_function_body_values_when_function_is_unset_before_first_call() {
        let source = "\
#!/bin/sh
cleanup() { unset -f fetch; }
version=v1
URL=\"https://example.invalid/$version\"
fetch() { echo $URL; }
cleanup
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$URL"]
        );
    }

    #[test]
    fn keeps_function_body_values_safe_when_function_is_called_before_later_unset() {
        let source = "\
#!/bin/sh
cleanup() { unset -f fetch; }
version=v1
URL=\"https://example.invalid/$version\"
fetch() { echo $URL; }
fetch
cleanup
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_function_body_values_when_indirect_unset_precedes_indirect_call() {
        let source = "\
#!/bin/sh
cleanup() { unset -f fetch; }
version=v1
URL=\"https://example.invalid/$version\"
fetch() { echo $URL; }
wrapper() { fetch; }
runner() {
  cleanup
  wrapper
}
runner
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$URL"]
        );
    }

    #[test]
    fn keeps_function_body_values_safe_when_indirect_call_precedes_indirect_unset() {
        let source = "\
#!/bin/sh
cleanup() { unset -f fetch; }
version=v1
URL=\"https://example.invalid/$version\"
fetch() { echo $URL; }
wrapper() { fetch; }
runner() {
  wrapper
  cleanup
}
runner
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_append_assignments_without_safe_prior_values() {
        let source = "\
#!/bin/bash
var+=ok
printf '%s\\n' $var
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$var"]
        );
    }

    #[test]
    fn skips_append_assignments_after_safe_prior_values() {
        let source = "\
#!/bin/bash
var=ok
var+=still_ok
printf '%s\\n' $var
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_multiple_conditional_appends_after_safe_prior_values() {
        let source = "\
#!/bin/bash
themes=gtk2,gtk3
type -P gnome-shell >/dev/null && themes+=,gnome-shell
type -P cinnamon-session >/dev/null && themes+=,cinnamon
meson -Dthemes=$themes
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_conditional_appends_after_empty_prior_values() {
        let source = "\
#!/bin/bash
options=
if [ \"$1\" = yes ]; then
  options+=--enable-experimental
fi
./configure $options
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_values_derived_from_unknown_append_assignments() {
        let source = "\
#!/bin/bash
f() {
  TERMUX_PKG_SRCDIR+=/Python
  TERMUX_PKG_BUILDDIR=\"$TERMUX_PKG_SRCDIR\"
  local _bindir=$TERMUX_PKG_BUILDDIR/_wrapper/bin
  mkdir -p ${_bindir}
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${_bindir}"]
        );
    }

    #[test]
    fn skips_append_assignments_to_numeric_shell_variables() {
        let source = "\
#!/bin/bash
SECONDS+=1
printf '%s\\n' $SECONDS
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_initialized_declaration_bindings_with_safe_values() {
        let source = "\
#!/bin/bash
f() {
  local name=abc i=0
  printf '%s\\n' $name $i
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_optional_safe_assignments_after_name_only_declarations() {
        let source = "\
#!/bin/bash
f() {
  local extra
  if true; then
    extra=EXTRA=-DPLAT_LINUX_RPI
  fi
  make $extra
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_mixed_empty_and_safe_branch_aliases() {
        let source = "\
#!/bin/bash
SRCNAM64=foo
SRCNAM32=
COMPRESS=deb
if [ \"$ARCH\" = i586 ]; then
  SRCNAM=\"$SRCNAM32\"
elif [ \"$ARCH\" = x86_64 ]; then
  SRCNAM=\"$SRCNAM64\"
else
  SRCNAM=
fi
ar x \"$CWD\"/$SRCNAM.$COMPRESS
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_all_empty_branch_aliases() {
        let source = "\
#!/bin/bash
empty=
if [ \"$1\" = yes ]; then
  value=$empty
else
  value=
fi
printf '%s\\n' $value
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$value"]
        );
    }

    #[test]
    fn reports_name_only_declarations_without_safe_assignments() {
        let source = "\
#!/bin/bash
f() {
  local extra
  make $extra
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$extra"]
        );
    }

    #[test]
    fn skips_safe_assignment_or_unset_branches() {
        let source = "\
#!/bin/bash
if [ \"$GTK2\" = enable ]; then
  GTK2=--with-gtk2
else
  unset GTK2
fi
./configure $GTK2
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_values_after_dominating_unsets() {
        let source = "\
#!/bin/bash
opt=-n
unset opt
printf '%s\\n' $opt
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$opt"]
        );
    }

    #[test]
    fn skips_safe_literal_bindings_inside_nested_command_substitutions() {
        let source = "\
#!/bin/bash
URL=https://example.com/file.tgz
FILE=$(basename $URL)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_local_literal_arguments_when_called_helper_reuses_name() {
        let source = "\
#!/bin/bash
build_payload() {
  service=$1
  version=$2
}
send_request() {
  action=$1
  service=alpha
  version=2024-01
  token=$(build_payload $service $version \"$action\")
}
send_request \"$1\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_dynamic_arguments_when_called_helper_reuses_name() {
        let source = "\
#!/bin/bash
build_payload() {
  service=$1
}
send_request() {
  service=$1
  token=$(build_payload $service)
}
send_request \"$1\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$service"]
        );
    }

    #[test]
    fn skips_local_literal_command_arguments_when_helper_reuses_name() {
        let source = "\
#!/bin/bash
config() {
  NEW=\"$1\"
  OLD=\"$(dirname $NEW)/$(basename $NEW .new)\"
}
config_blacklist() {
  NEW=etc/app/blacklist
  OLD=etc/app/package_blacklist
  cp $OLD $NEW
}
config \"$1\"
config_blacklist
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$NEW", "$NEW"]
        );
    }

    #[test]
    fn reports_dynamic_command_arguments_when_helper_reuses_name() {
        let source = "\
#!/bin/bash
config() {
  NEW=etc/app/default
}
config_blacklist() {
  NEW=\"$1\"
  OLD=etc/app/package_blacklist
  cp $OLD $NEW
}
config
config_blacklist \"$1\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$NEW"]
        );
    }

    #[test]
    fn skips_transitive_helper_composite_quote_inert_values() {
        let source = "\
#!/bin/bash
exit_script() { exit 1; }
default_settings() {
  FORMAT=\",efitype=4m\"
  DISK_CACHE=\"\"
}
advanced_settings() {
  if MACH=$(choose); then
    if [ \"$MACH\" = q35 ]; then
      FORMAT=\"\"
    else
      FORMAT=\",efitype=4m\"
    fi
  else
    exit_script
  fi
  if DISK_CACHE=$(choose); then
    if [ \"$DISK_CACHE\" = \"1\" ]; then
      DISK_CACHE=\"cache=writethrough,\"
    else
      DISK_CACHE=\"\"
    fi
  else
    exit_script
  fi
}
start_script() {
  if choose; then
    default_settings
  else
    advanced_settings
  fi
}
start_script
VMID=100
STORAGE=local
THIN=\"discard=on,ssd=1,\"
case \"$STORAGE\" in
  btrfs)
    FORMAT=\",efitype=4m\"
    ;;
esac
DISK0_REF=${STORAGE}:vm-${VMID}-disk-0
DISK1_REF=${STORAGE}:vm-${VMID}-disk-1
qm set $VMID \\
  -efidisk0 ${DISK0_REF}${FORMAT} \\
  -scsi0 ${DISK1_REF},${DISK_CACHE}${THIN}size=12G
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_transitive_helper_dynamic_composite_values() {
        let source = "\
#!/bin/bash
default_settings() {
  DISK_CACHE=$1
}
advanced_settings() {
  DISK_CACHE=\"cache=writethrough,\"
}
start_script() {
  if choose; then
    default_settings \"$1\"
  else
    advanced_settings
  fi
}
start_script \"$1\"
DISK1_REF=local:vm-100-disk-1
qm set 100 -scsi0 ${DISK1_REF},${DISK_CACHE}size=12G
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${DISK_CACHE}"]
        );
    }

    #[test]
    fn reports_top_level_branch_literal_without_dispatch_fallback() {
        let source = "\
#!/bin/bash
STORAGE=local
case \"$STORAGE\" in
  btrfs)
    FORMAT=\",efitype=4m\"
    ;;
esac
DISK0_REF=${STORAGE}:vm-100-disk-0
qm set 100 -efidisk0 ${DISK0_REF}${FORMAT}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${FORMAT}"]
        );
    }

    #[test]
    fn skips_safe_numeric_shell_variables() {
        let source = "\
#!/bin/bash
printf '%s\\n' $(ps -o comm= -p $PPID) $UID $EUID $RANDOM $OPTIND $SECONDS $LINENO $BASHPID $COLUMNS
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_numeric_shell_variables_after_default_assignment() {
        let source = "\
#!/bin/bash
: \"${UID:=1000}\"
usermod -u $UID minecraft
UID='a b'
usermod -u $UID minecraft
unset COLUMNS
: \"${COLUMNS:=$1}\"
printf '%s\\n' $COLUMNS
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$UID", "$COLUMNS"]
        );
    }

    #[test]
    fn skips_loop_carried_numeric_default_operands() {
        let source = "\
#!/bin/sh
encode() {
  local text=\"$1\"
  local pos char
  while [ ${pos:-0} -lt ${#text} ]; do
    pos=$(( pos + 1 ))
    char=$(printf '%s' \"$text\" | cut -b $pos)
    printf '%s' \"$char\"
  done
}
scan() {
  local count=0 overall file_list
  file_list=$(printf '%s\\n' item)
  while [ -n \"$file_list\" -a $count -le ${overall:-$count} ]; do
    for item in $file_list; do count=$(( count + 1 )); done
    overall=$count
  done
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_file_scope_integer_bindings_in_indirectly_called_numeric_tests() {
        let source = "\
#!/usr/bin/env bash
scan() { regular_grep \"$@\"; }
regular_grep() {
  [ ${RECURSIVE} -eq 1 ] && action=recurse
}
run_named() {
  local scan_fn=\"$1\"
  shift
  $scan_fn \"$@\"
}
declare COMMAND=\"$1\" RECURSIVE=0
while [ \"$#\" -gt 0 ]; do
  case \"$1\" in
    -r) RECURSIVE=1 ;;
  esac
  shift
done
case \"$COMMAND\" in
  --scan) run_named scan \"$@\" ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_file_scope_integer_bindings_after_function_calls() {
        let source = "\
#!/bin/bash
check_count() {
  [ $count -eq 0 ] && :
}
check_count
count=0
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$count"]
        );
    }

    #[test]
    fn skips_file_scope_integer_bindings_before_function_calls() {
        let source = "\
#!/bin/bash
count=0
check_count() {
  [ $count -eq 0 ] && :
}
check_count
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_counter_arguments_with_only_numeric_assignments() {
        let source = "\
#!/usr/bin/env bash
update_count_column_width() {
  count_column_width=$((${#count} * 2 + 2))
  update_count_column_left
}
update_screen_width() {
  screen_width=\"$(tput cols)\"
  update_count_column_left
}
update_count_column_left() {
  count_column_left=$((screen_width - count_column_width))
}
count=0
screen_width=80
update_count_column_width
begin() {
  line_backoff_count=0
  update_count_column_width
  go_to_column $count_column_left
}
finish_test() {
  move_up $line_backoff_count
}
line_backoff_count=0
bats_tap_stream_comment() {
  ((++line_backoff_count))
  ((line_backoff_count += ${#1} / screen_width))
}
dynamic_test() {
  quiet=${1:-0}
  if [ ${quiet} -eq 0 ]; then :; fi
}
static_status() {
  ret=0
  command && ret=1 || ret=0
  tend ${ret}
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${quiet}"]
        );
    }

    #[test]
    fn skips_arithmetic_numeric_test_operands_without_static_flag_broadening() {
        let source = "\
#!/bin/sh
filetime=$(date +%s -r \"$file\")
fileage=$(( unixtime - filetime ))
[ $fileage -lt 86400 ] && :
quiet=${1:-0}
[ ${quiet} -eq 0 ] && :
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${quiet}"]
        );
    }

    #[test]
    fn reports_numeric_defaults_without_loop_carried_updates() {
        let source = "\
#!/bin/sh
plain() {
  local pos
  [ ${pos:-0} -lt 3 ] && :
}
dynamic_update() {
  local pos
  while [ ${pos:-0} -lt 3 ]; do
    pos=$1
  done
}
dynamic_default() {
  local pos fallback
  while [ ${pos:-$fallback} -lt 3 ]; do
    pos=$(( pos + 1 ))
  done
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${pos:-0}", "${pos:-0}", "${pos:-$fallback}"]
        );
    }

    #[test]
    fn reports_unknown_values_in_uncalled_function_bodies() {
        let source = "\
#!/bin/sh
unused() {
  echo $1 $value
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$1", "$value"]
        );
    }

    #[test]
    fn keeps_called_function_arguments_unsafe() {
        let source = "\
#!/bin/sh
called() {
  echo $1 $value
}
called \"$@\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$1", "$value"]
        );
    }

    #[test]
    fn skips_status_capture_bindings_with_local_declarations() {
        let source = "\
#!/bin/bash
function first() {
  local return_value
  other \"$@\"
  return_value=$?
  if [ $return_value -eq 64 ]; then :; fi
  return $return_value
}
second() {
  local return_value
  for x in \"$@\"; do
    y=$(cat \"$x\")
    return_value=$?
    if [ $return_value -ne 0 ]; then return $return_value; fi
  done
}
first \"$@\"
second \"$@\"
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_nested_local_status_capture_returns() {
        let source = "\
#!/bin/sh
prompt_set() {
  face() {
    local rc=$?

    case \"$rc\" in
      0) printf '%s' \"$1\" ;;
      *) printf '%s' \"$2\" ; return $rc ;;
    esac
  }
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_name_only_local_option_bindings() {
        let source = "\
#!/bin/bash
request() {
  local header_opt data_opt
  [[ -n $token ]] && header_opt=--header
  [[ $method == POST ]] && data_opt=--data
  curl $header_opt \"$header\" $data_opt \"$payload\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_local_conditional_literal_option_letters() {
        let source = "\
#!/bin/bash
read_char() {
  local read_flag anykey
  [[ -n ${ZSH_VERSION:-} ]] && read_flag=k || read_flag=n
  builtin read -${read_flag} 1 -s -r anykey
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_local_conditional_dynamic_option_letters() {
        let source = "\
#!/bin/bash
read_char() {
  local read_flag anykey
  [[ -n ${ZSH_VERSION:-} ]] && read_flag=$1 || read_flag=n
  builtin read -${read_flag} 1 -s -r anykey
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${read_flag}"]
        );
    }

    #[test]
    fn skips_status_capture_declarations_with_initializers() {
        let source = "\
#!/bin/bash
cleanup() {
  rm -f -- \"$1\" || {
    \\typeset ret=$?
    return $ret
  }
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_escaped_declaration_literal_return_operands() {
        let source = "\
#!/bin/bash
cleanup() {
  \\typeset __result=0
  run_task \"$@\" || __result=$?
  return ${__result}
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_escaped_declaration_dynamic_return_operands() {
        let source = "\
#!/bin/bash
cleanup() {
  \\typeset __result=$1
  return ${__result}
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${__result}"]
        );
    }

    #[test]
    fn skips_subshell_status_return_operands_with_safe_base() {
        let source = "\
#!/bin/bash
cleanup()
(
  \\typeset __result
  __result=0
  run_task \"$@\" || __result=$?
  next_step && final_step || __result=$?
  return ${__result}
)
invoke_cleanup() {
  cleanup \"$@\"
}
invoke_cleanup \"$@\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_pid_capture_bindings() {
        let source = "\
#!/bin/bash
long_running_task &
tarpid=$!
counter=$(ps -A | grep $tarpid | wc -l)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_exit_arguments_with_only_numeric_bindings() {
        let source = "\
#!/bin/bash
exit_code=1
if [[ \"$1\" = retry ]]; then
  exit_code=2
fi
exit ${exit_code}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_status_capture_after_escaped_name_only_declarations() {
        let source = "\
#!/bin/bash
cleanup() {
  maybe_helper \"$1\"
  \\typeset result other
  false
  result=$?
  if (( result > 0 )); then
    return $result
  fi
  return ${result:-0}
}
maybe_helper() {
  \\typeset result
  if [[ -n \"$1\" ]]; then
    result=$?
    return $result
  fi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion).with_resolve_source_closure(false),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_status_capture_after_mixed_static_and_escaped_declaration_bindings() {
        let source = "\
#!/bin/bash
download_the_url() {
  result=0
  curl \"$url\" && mv file file.part || {
    result=$?
    case \"$result\" in
      18) download_the_url \"$counter\" ;;
      *) ;;
    esac
    return $result
  }
}
download_the_url || {
  \\typeset __fallback __default_url __default __iterator result=$?
  if (( result )); then
    fail \"no fallback\" $result
  fi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion).with_resolve_source_closure(false),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_status_capture_declarations_after_unsafe_reassignments() {
        let source = "\
#!/bin/bash
demo() {
  local ret=$?
  ret=$user_input
  echo $ret
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$ret"]
        );
    }

    #[test]
    fn reports_parameters_shadowing_outer_status_captures() {
        let source = "\
#!/bin/bash
status=$?
report() {
  local status=\"$2\"
  if [ $status -eq 1 ]; then :; fi
}
report \"$@\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$status"]
        );
    }

    #[test]
    fn skips_arithmetic_and_escaped_declaration_assignment_arguments() {
        let source = "\
#!/bin/bash
let COOLDOWN=$AUTOPAUSE_TIMEOUT_KN/2
\\typeset counter=${1:-0}
\\typeset __action=$1
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn uses_static_loop_values_from_static_function_call_sites() {
        let source = "\
#!/bin/bash
run() {
  LDFLAGS=\"-fuse-ld=${linker}\"
  cc ${CFLAGS} ${LDFLAGS}
}
for linker in gold bfd lld; do
  run
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CFLAGS}"]
        );
    }

    #[test]
    fn uses_static_loop_values_after_prior_local_declarations() {
        let source = "\
#!/bin/bash
run() {
  LDFLAGS=\"-fuse-ld=${linker}\"
  cc ${CFLAGS} ${LDFLAGS}
}
main() {
  local linker
  for linker in gold bfd lld; do
    run
  done
}
main
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CFLAGS}"]
        );
    }

    #[test]
    fn uses_static_loop_values_through_intermediate_helpers() {
        let source = "\
#!/bin/bash
run() {
  LDFLAGS=\"-fuse-ld=${linker}\"
  cc ${CFLAGS} ${LDFLAGS}
}
dispatch() {
  run
}
main() {
  local linker
  for linker in gold bfd lld; do
    dispatch
  done
}
main
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CFLAGS}"]
        );
    }

    #[test]
    fn reports_static_loop_values_when_intermediate_helpers_have_unsafe_callers() {
        let source = "\
#!/bin/bash
run() {
  LDFLAGS=\"-fuse-ld=${linker}\"
  cc ${CFLAGS} ${LDFLAGS}
}
dispatch() {
  run
}
main() {
  local linker
  for linker in gold bfd lld; do
    dispatch
  done
}
main
dispatch
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CFLAGS}", "${LDFLAGS}"]
        );
    }

    #[test]
    fn uses_static_loop_call_values_over_unrelated_prior_loop_bindings() {
        let source = "\
#!/bin/bash
for linker in gold bfd lld; do
  :
done
run() {
  LDFLAGS=\"-fuse-ld=${linker}\"
  cc ${CFLAGS} ${LDFLAGS}
}
main() {
  local linker
  for linker in gold bfd lld; do
    [[ ${CC} == gcc && ${linker} == lld ]] && continue
    LDFLAGS=\"-fuse-ld=${linker}\" tc-ld-is-${linker} || continue
    run
  done
}
main
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CFLAGS}"]
        );
    }

    #[test]
    fn reports_values_from_guarded_safe_assignments_on_uncovered_paths() {
        let source = "\
#!/bin/bash
source \"$CONFIG\"
[[ -z $folder || ! -w $(dirname \"$folder\") ]] && folder=~/gist
mkdir -p $folder
find $folder -maxdepth 1
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$folder", "$folder"]
        );
    }

    #[test]
    fn reports_helper_values_from_one_sided_short_circuit_assignments() {
        let source = "\
#!/bin/bash
source \"$CONFIG\"
init_folder() {
  [[ -z $folder ]] && folder=~/gist
}
init_folder
find $folder -maxdepth 1
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$folder"]
        );
    }

    #[test]
    fn skips_one_sided_short_circuit_assignments_after_covering_safe_values() {
        let source = "\
#!/bin/bash
folder=/var
[[ -d x ]] && folder=/tmp
find $folder -maxdepth 1
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn uses_safe_top_level_bindings_at_static_function_call_sites() {
        let source = "\
#!/bin/sh
RETVAL=0
prog=daemon
start() {
  if [ $RETVAL -eq 0 ]; then
    touch /var/lock/subsys/$prog
  fi
  return $RETVAL
}
case \"$1\" in
  start) start ;;
esac
exit $RETVAL
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn uses_safe_top_level_bindings_through_static_dispatch_helpers() {
        let source = "\
#!/bin/sh
RETVAL=0
prog=daemon
start () {
  RETVAL=$?
  if [ $RETVAL -eq 0 ]; then
    touch /var/lock/subsys/$prog
  fi
  return $RETVAL
}
restart () {
  start
}
case \"$1\" in
  start) start ;;
  restart) restart ;;
esac
exit $RETVAL
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_status_capture_helpers_called_from_multiple_case_arms() {
        let source = "\
#!/bin/sh
RETVAL=0
start() {
  return 0
  RETVAL=$?
  if [ $RETVAL -eq 0 ]; then :; fi
  return $RETVAL
}
stop() {
  RETVAL=$?
  if [ $RETVAL -eq 0 ]; then :; fi
  return $RETVAL
}
case \"$1\" in
  start) start ;;
  stop) stop ;;
  restart)
    stop
    start
    ;;
  *) RETVAL=1 ;;
esac
exit $RETVAL
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![(6, 8, 6, 15), (7, 10, 7, 17)]
        );
    }

    #[test]
    fn reports_reassigned_ppid_in_sh_mode() {
        let source = "\
#!/bin/sh
PPID='a b'
printf '%s\\n' $PPID
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/pkg.sh"),
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$PPID"]
        );
    }

    #[test]
    fn skips_safe_here_string_operands() {
        let source = "\
#!/bin/bash
URL=https://example.com/file.tgz
cat <<< $URL
cat <<< $PPID
v='a b'
cat <<< $v
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$v"]
        );
    }

    #[test]
    fn skips_safe_literal_loop_variables() {
        let source = "\
#!/bin/bash
for v in one two; do
  unset $v
done
for i in 16 32 64; do
  cmd ${i}x${i}! \"$i\"
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_static_loop_variables_after_the_loop_body() {
        let source = "\
#!/bin/bash
for i in castool chdman; do
  :
done
install $i /tmp
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$i"]
        );
    }

    #[test]
    fn reports_loop_variables_derived_from_expanded_values() {
        let source = "\
#!/bin/bash
PRGNAM=neverball
BONUS=neverputt
for i in $PRGNAM $BONUS; do
  install -D ${i}.desktop /tmp/$i.png
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${i}", "$i"]
        );
    }

    #[test]
    fn reports_loop_variables_derived_from_at_slices() {
        let source = "\
#!/bin/bash
f() {
  for v in ${@:2}; do
    del $v
  done
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$v"]
        );
    }

    #[test]
    fn skips_direct_at_slices_that_belong_to_array_split_handling() {
        let source = "\
#!/bin/bash
f() {
  dns_set ${@:2}
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_bindings_derived_from_arithmetic_values() {
        let source = "\
#!/bin/bash
x=$((1 + 2))
y=\"$x\"
z=${x}
printf '%s\\n' $x $y $z
if [ $x -eq 0 ]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn skips_safe_assign_default_expansions_when_name_is_already_safe() {
        let source = "\
#!/bin/bash
check_count() {
  local i=0
  [ ${i:=0} -eq 0 ]
  test $i -ge 0
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_default_operator_operands_when_existing_value_is_safe() {
        let source = "\
#!/bin/bash
value=abc
printf '%s\\n' ${value:=$1} ${value:-$2}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_default_operator_operands_when_safe_value_can_be_empty() {
        let source = "\
#!/bin/bash
if cond; then use=\"\"; else use=ok; fi
if cond; then assign=\"\"; else assign=ok; fi
if cond; then err=\"\"; else err=ok; fi
printf '%s\\n' ${use:-$1} ${assign:=$2} ${err:?$3}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_assign_default_after_maybe_uninitialized_binding() {
        let source = "\
#!/bin/bash
if cond; then value=abc; fi
printf '%s\\n' ${value:=fallback}
printf '%s\\n' $value
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${value:=fallback}", "$value"]
        );
    }

    #[test]
    fn reports_conditionally_sanitized_test_operands() {
        let source = "\
#!/bin/bash
foo=$BAR
if [ \"$foo\" = \"\" ]; then
  foo=0
fi
if [ $foo -eq 1 ]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$foo"]
        );
    }

    #[test]
    fn skips_status_capture_numeric_test_operands() {
        let source = "\
#!/bin/bash
f() {
  local status
  run_task && status=0 || status=$?
  if [ $status -eq 0 ]; then :; fi
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_declared_integer_default_numeric_test_operands() {
        let source = "\
#!/bin/bash
declare -i expected_status
expected_status=${expected_status:-0}
actual_status=$1
if [ ${actual_status} -ne ${expected_status} ]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${actual_status}"]
        );
    }

    #[test]
    fn skips_plain_run_first_argument() {
        let source = "\
#!/bin/bash
target=$1
run $target
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_plain_run_later_arguments() {
        let source = "\
#!/bin/bash
target=$1
run echo $target
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$target"]
        );
    }

    #[test]
    fn reports_wrapped_run_first_arguments() {
        let source = "\
#!/bin/bash
target=$1
command run $target
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$target"]
        );
    }

    #[test]
    fn reports_conditionally_initialized_bindings_with_unknown_fallbacks() {
        let source = "\
#!/bin/bash
if [ \"$1\" = yes ]; then
  foo=0
fi
printf '%s\\n' $foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$foo"]
        );
    }

    #[test]
    fn skips_uncalled_function_references_after_safe_global_initialization() {
        let source = "\
#!/bin/bash
value=\"$1\"
helper() { printf '%s\\n' $value; }
value=abc
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_uncalled_function_references_after_unsafe_global_initialization() {
        let source = "\
#!/bin/bash
value=abc
helper() { printf '%s\\n' $value; }
value=\"$1\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$value"]
        );
    }

    #[test]
    fn reports_uncalled_function_references_after_guarded_unsafe_global_update() {
        let source = "\
#!/bin/bash
value=abc
if cond; then value=\"$1\"; fi
helper() { printf '%s\\n' $value; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$value"]
        );
    }

    #[test]
    fn reports_uncalled_function_references_after_partial_global_initialization() {
        let source = "\
#!/bin/bash
helper() { printf '%s\\n' $value; }
if [ \"$1\" = yes ]; then
  value=abc
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$value"]
        );
    }

    #[test]
    fn skips_straight_line_safe_overwrites_in_test_operands() {
        let source = "\
#!/bin/bash
foo=$BAR
foo=0
if [ $foo -eq 1 ]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_case_arm_safe_overwrites_in_test_operands() {
        let source = "\
#!/bin/bash
foo=$BAR
case $1 in
  settings)
    foo=0
    if [ $foo -eq 1 ]; then :; fi
    ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_case_arm_safe_overwrites_even_with_nested_conditional_updates() {
        let source = "\
#!/bin/bash
foo=$BAR
case $1 in
  settings)
    foo=1
    while [ $# -gt 1 ]; do
      shift
      case $1 in
        --no) foo=0 ;;
      esac
    done
    if [ $foo -eq 1 ]; then :; fi
    ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_if_else_safe_literal_bindings() {
        let source = "\
#!/bin/bash
if [ \"$1\" = h ]; then
  humanreadable=-h
else
  humanreadable=-m
fi
free ${humanreadable}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_if_else_safe_literal_bindings_inside_command_substitutions() {
        let source = "\
#!/bin/bash
if [ \"$1\" = h ]; then
  humanreadable=-h
else
  humanreadable=-m
fi
value=\"$(free ${humanreadable} | awk '{print $2}')\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_safe_helper_initialized_option_flags_after_intermediate_calls() {
        let source = "\
#!/bin/bash
fn_select_compression() {
  if command -v zstd >/dev/null 2>&1; then
    compressflag=--zstd
  elif command -v pigz >/dev/null 2>&1; then
    compressflag=--use-compress-program=pigz
  elif command -v gzip >/dev/null 2>&1; then
    compressflag=--gzip
  else
    compressflag=
  fi
}

fn_backup_check_lockfile() { :; }
fn_backup_create_lockfile() { :; }
fn_backup_init() { :; }
fn_backup_stop_server() { :; }
fn_backup_dir() { :; }

fn_backup_compression() {
  if [ -n \"${compressflag}\" ]; then
    tar ${compressflag} -hcf out.tar ./.
  else
    tar -hcf out.tar ./.
  fi
}

fn_select_compression
fn_backup_check_lockfile
fn_backup_create_lockfile
fn_backup_init
fn_backup_stop_server
fn_backup_dir
fn_backup_compression
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_safe_option_flags_after_explicit_unset_baseline() {
        let source = "\
#!/bin/bash
unset keep
unset pipeline_ref
unset pipe_keep | cat
(unset subshell_keep)
unset -n nameref_keep
unset empty_only
if [ \"$1\" = yes ]; then
  keep=-k
fi
if [ \"$1\" = ref ]; then
  pipeline_ref=-k
fi
if [ \"$1\" = pipe ]; then
  pipe_keep=-k
fi
if [ \"$1\" = sub ]; then
  subshell_keep=-k
fi
if [ \"$1\" = name ]; then
  nameref_keep=-k
fi
if [ \"$1\" = no ]; then
  empty_only=
fi
if [ \"$2\" = yes ]; then
  missing=-v
fi
python-build $keep $pipe_keep $subshell_keep $nameref_keep $empty_only $missing
python-build $pipeline_ref | cat
unset only
python-build $only
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$pipe_keep",
                "$subshell_keep",
                "$nameref_keep",
                "$empty_only",
                "$missing",
                "$only"
            ]
        );
    }

    #[test]
    fn reports_top_level_arguments_after_transitively_unsafe_helper_calls() {
        let source = "\
#!/bin/bash
DISK_SIZE=\"32G\"

advanced_settings() {
  DISK_SIZE=\"$(get_size)\"
}

start_script() {
  advanced_settings
}

start_script
if [ -n \"$DISK_SIZE\" ]; then
  qm resize 100 scsi0 ${DISK_SIZE} >/dev/null
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${DISK_SIZE}"]
        );
    }

    #[test]
    fn reports_top_level_arguments_after_branch_unsafe_helper_calls() {
        let source = "\
#!/bin/bash
DISK_SIZE=\"32G\"

advanced_settings() {
  DISK_SIZE=\"$(get_size)\"
}

start_script() {
  if choose; then
    :
  else
    advanced_settings
  fi
}

start_script
qm resize 100 scsi0 ${DISK_SIZE}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${DISK_SIZE}"]
        );
    }

    #[test]
    fn reports_top_level_arguments_after_recursive_unsafe_helper_calls() {
        let source = "\
#!/bin/bash
DISK_SIZE=\"32G\"

advanced_settings() {
  if choose_again; then
    advanced_settings
  fi
  DISK_SIZE=\"$(get_size)\"
}

start_script() {
  if choose; then
    :
  else
    advanced_settings
  fi
}

start_script
qm resize 100 scsi0 ${DISK_SIZE}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${DISK_SIZE}"]
        );
    }

    #[test]
    fn reports_local_bindings_initialized_via_called_helpers() {
        let source = "\
#!/bin/bash
setup() {
  mode=NTSC
}

render() {
  printf '%s\\n' $mode
}

main() {
  local mode
  setup
  render
  printf '%s\\n' $mode
}

main
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$mode", "$mode"]
        );
    }

    #[test]
    fn reports_helper_initialized_bindings_when_other_callers_skip_the_helper() {
        let source = "\
#!/bin/bash
init_flag() {
  flag=-n
}

render() {
  printf '%s\\n' ${flag}
}

safe_path() {
  init_flag
  render
}

unsafe_path() {
  render
}

safe_path
unsafe_path
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${flag}"]
        );
    }

    #[test]
    fn reports_helper_globals_with_unsafe_caller_visible_bindings() {
        let source = "\
#!/bin/sh
SERVERNUM=99
find_free_servernum() {
  i=$SERVERNUM
  while [ -f /tmp/.X$i-lock ]; do
    i=$(($i + 1))
  done
  echo $i
}
set -- -n '1 2' -a --
while :; do
  case \"$1\" in
    -a|--auto-servernum) SERVERNUM=$(find_free_servernum) ;;
    -n|--server-num) SERVERNUM=\"$2\"; shift ;;
    --) shift; break ;;
    *) break ;;
  esac
  shift
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$i", "$i"]
        );
    }

    #[test]
    fn reports_called_helper_mutations_even_when_callers_have_safe_bindings() {
        let source = "\
#!/bin/bash
flag=-n
set_flag() {
  flag=$1
}
render() {
  set_flag \"$1\"
  printf '%s\\n' $flag
}
render value
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$flag"]
        );
    }

    #[test]
    fn reports_helper_globals_with_mixed_safe_and_unsafe_caller_branches() {
        let source = "\
#!/bin/sh
if [ -n \"$2\" ]; then
  UIPORT=\"$2\"
else
  UIPORT=\"8080\"
fi
do_start() {
  grep $UIPORT /dev/null
}
do_start
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$UIPORT"]
        );
    }

    #[test]
    fn reports_helper_globals_with_conditionally_initialized_caller_bindings() {
        let source = "\
#!/bin/bash
[ \"$1\" = 64 ] && extra=ENABLE_LIB64=1
run_make() {
  make $extra
}
run_make
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$extra"]
        );
    }

    #[test]
    fn reports_helper_bindings_when_initializers_are_guarded_by_conditionals() {
        let source = "\
#!/bin/bash
init_flag() {
  flag=-n
}

render() {
  if [ \"$1\" = yes ]; then
    init_flag
  fi
  printf '%s\\n' ${flag}
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${flag}"]
        );
    }

    #[test]
    fn skips_helper_initialized_bindings_when_all_callers_provide_distinct_values() {
        let source = "\
#!/bin/bash
init_flag_a() {
  flag=-a
}

init_flag_b() {
  flag=-b
}

render() {
  printf '%s\\n' ${flag}
}

safe_path_a() {
  init_flag_a
  render
}

safe_path_b() {
  init_flag_b
  render
}

safe_path_a
safe_path_b
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_tilde_path_bindings_shellcheck_treats_as_field_safe() {
        let source = "\
#!/bin/bash
file=~/.launch-bristol
if [ -e $file ]; then
  dflt=\"$(cat $file)\"
fi
> $file.new
mv $file.new $file
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_self_referential_static_suffix_bindings() {
        let path = Path::new("/tmp/example.SlackBuild");
        let source = "\
#!/bin/bash
LIBDIR=lib
if [ \"$ARCH\" = \"x86_64\" ]; then
  LIBDIR=\"$LIBDIR\"64
fi
mkdir -p /pkg/usr/$LIBDIR/clap
";
        let diagnostics = test_snippet_at_path(
            path,
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_status_bindings_when_local_initializer_covers_helper_updates() {
        let source = "\
#!/bin/sh
deploy() {
  err_code=0
  if ! remote; then
    return $err_code
  fi
}

remote() {
  err_code=$?
  return $err_code
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_safe_bindings_after_exit_inside_nested_command_substitution() {
        let source = "\
#!/bin/bash
name=package-name
version=1
list=$(echo file || (echo error; exit 1))
mkdir -p /pkg/usr/doc/$name-$version
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_exhaustive_static_branch_bindings_inside_brace_expansion() {
        let source = "\
#!/bin/bash
if [ \"$ARCH\" = \"i586\" ]; then
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"x86_64\" ]; then
  LIBDIRSUFFIX=\"64\"
else
  LIBDIRSUFFIX=\"\"
fi
mkdir -p /pkg/usr/{bin,lib${LIBDIRSUFFIX}}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_definite_empty_overwrite_inside_brace_expansion() {
        let source = "\
#!/bin/sh
x=64
x=
echo {pre,$x}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$x"]
        );
    }

    #[test]
    fn reports_ambient_contract_bindings_without_known_values() {
        let path = Path::new("/tmp/void-packages/common/build-style/example.sh");
        let source = "\
#!/bin/sh
helper() {
  printf '%s\\n' $wrksrc $pkgname
}
";
        let diagnostics = test_snippet_at_path(
            path,
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$wrksrc", "$pkgname"]
        );
    }

    #[test]
    fn skips_static_suffix_bindings_in_slackbuild_subshell_paths() {
        let path = Path::new("/tmp/example.SlackBuild");
        let source = "\
#!/bin/bash
if [ \"$ARCH\" = \"i386\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"i486\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"i586\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"i686\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"x86_64\" ]; then
  MULTILIB=\"YES\"
  LIBDIRSUFFIX=\"64\"
elif [ \"$ARCH\" = \"armv7hl\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"s390\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
else
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
fi

if [ ${MULTILIB} = \"YES\" ]; then
  printf '%s\\n' multilib
fi

(
  ./configure \
    --libdir=/usr/lib${LIBDIRSUFFIX} \
    --with-python-dir=/lib${LIBDIRSUFFIX}/python2.7/site-packages \
    --with-java-home=/usr/lib${LIBDIRSUFFIX}/jvm/jre
)
";
        let diagnostics = test_snippet_at_path(
            path,
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_static_suffix_bindings_in_realistic_slackbuild_path_words() {
        let path = Path::new("/tmp/example.SlackBuild");
        let source = "\
#!/bin/bash
if [ -z \"$ARCH\" ]; then
  ARCH=$(uname -m)
  export ARCH
fi

if [ \"$ARCH\" = \"x86_64\" ]; then
  MULTILIB=\"YES\"
else
  MULTILIB=\"NO\"
fi

if [ \"$ARCH\" = \"i386\" ]; then
  LIBDIRSUFFIX=\"\"
  LIB_ARCH=i386
elif [ \"$ARCH\" = \"i486\" ]; then
  LIBDIRSUFFIX=\"\"
  LIB_ARCH=i386
elif [ \"$ARCH\" = \"i586\" ]; then
  LIBDIRSUFFIX=\"\"
  LIB_ARCH=i386
elif [ \"$ARCH\" = \"i686\" ]; then
  LIBDIRSUFFIX=\"\"
  LIB_ARCH=i386
elif [ \"$ARCH\" = \"s390\" ]; then
  LIBDIRSUFFIX=\"\"
  LIB_ARCH=s390
elif [ \"$ARCH\" = \"x86_64\" ]; then
  LIBDIRSUFFIX=\"64\"
  LIB_ARCH=amd64
elif [ \"$ARCH\" = \"armv7hl\" ]; then
  LIBDIRSUFFIX=\"\"
  LIB_ARCH=armv7hl
else
  LIBDIRSUFFIX=\"\"
  LIB_ARCH=$ARCH
fi

(
  ./configure \
    --prefix=/usr \
    --libdir=/usr/lib$LIBDIRSUFFIX \
    --with-python-dir=/lib$LIBDIRSUFFIX/python2.7/site-packages \
    --with-java-home=/usr/lib$LIBDIRSUFFIX/jvm/jre \
    --with-jvm-root-dir=/usr/lib$LIBDIRSUFFIX/jvm \
    --with-jvm-jar-dir=/usr/lib$LIBDIRSUFFIX/jvm/jvm-exports \
    --with-arch-directory=$LIB_ARCH
)

if [ ! -r /pkg/usr/lib${LIBDIRSUFFIX}/gcc/x/y/specs ]; then
  cat stage1-gcc/specs > /pkg/usr/lib${LIBDIRSUFFIX}/gcc/x/y/specs
fi
if [ -d /pkg/usr/lib${LIBDIRSUFFIX} ]; then
  mv /pkg/usr/lib${LIBDIRSUFFIX}/lib* /pkg/usr/lib${LIBDIRSUFFIX}/gcc/x/y/
fi
";
        let settings = LinterSettings::for_rule(Rule::UnquotedExpansion)
            .with_analyzed_paths([path.to_path_buf()]);
        let diagnostics = test_snippet_at_path(path, source, &settings);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$LIB_ARCH"]
        );
    }

    #[test]
    fn keeps_safe_indirect_bindings_but_reports_parameter_operator_results() {
        let source = "\
#!/bin/bash
base=abc
name=base
upper=${base^^}
value='a b*'
quoted=${value@Q}
printf '%s\\n' ${!name} $upper $quoted
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$upper", "$quoted"]
        );
    }

    #[test]
    fn indirect_cycles_and_multi_field_targets_stay_unsafe() {
        let source = "\
#!/bin/bash
split='1 2'
name=split
a=$b
b=$a
printf '%s\\n' ${!name} $a
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$a"]
        );
    }

    #[test]
    fn indirect_expansions_follow_visible_name_safety() {
        let source = "\
#!/bin/bash
target='a b'
name=target
printf '%s\\n' ${!name}
for loop_name in target unset_name; do
  printf '%s\\n' ${!loop_name}
done
dynamic=$1
printf '%s\\n' ${!dynamic}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${!dynamic}"]
        );
    }

    #[test]
    fn skips_plain_unquoted_scalars_in_native_zsh_mode() {
        let source = "print $name\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_unquoted_scalars_after_setopt_sh_word_split_in_zsh() {
        let source = "setopt sh_word_split\nprint $name\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$name"]
        );
    }

    #[test]
    fn reports_zsh_force_split_modifier_even_without_sh_word_split() {
        let source = "print ${=name}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${=name}"]
        );
    }

    #[test]
    fn skips_zsh_double_tilde_modifier_when_it_forces_globbing_off() {
        let source = "print ${~~name}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
