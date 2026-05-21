use super::case_layout::{
    case_close_shares_line_with_last_item, case_command_was_inline_in_source,
    case_item_body_was_inline_without_terminator,
    case_item_pattern_body_terminator_was_inline_in_source,
    case_item_pattern_starts_on_case_header,
};
use super::shape::{
    can_inline_body_with_upper_bound, can_inline_else_branch_close, can_inline_if_chain,
    then_branch_starts_with_inline_if,
};
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IfLayoutPlan {
    pub(super) then_span: Span,
    pub(super) fi_span: Span,
    pub(super) style: IfLayoutStyle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum IfLayoutStyle {
    RawGroupedCondition { raw_condition: String },
    SplitCondition,
    Inline(InlineIfLayout),
    Expanded(ExpandedIfLayout),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InlineIfLayout {
    Then,
    ThenElse,
    ThenMultilineElse,
    ThenNestedIf,
    Chain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExpandedIfLayout {
    Compact,
    Multiline { inline_else_close: bool },
}

impl IfLayoutPlan {
    pub(super) fn then_fi(command: &IfCommand, context: RenderContext<'_, '_>) -> Self {
        let IfSyntax::ThenFi { then_span, .. } = command.syntax else {
            unreachable!("brace if cannot be planned as then/fi");
        };
        let fi_span = command_if_close_span(command, context.source, context.source_map());
        let style = then_fi_if_style(command, then_span, fi_span, context);

        Self {
            then_span,
            fi_span,
            style,
        }
    }
}

fn then_fi_if_style(
    command: &IfCommand,
    then_span: Span,
    fi_span: Span,
    context: RenderContext<'_, '_>,
) -> IfLayoutStyle {
    let fi_upper_bound = fi_span.start.offset;
    let no_elifs = command.elif_branches.is_empty();

    if no_elifs
        && let Some(raw_condition) = raw_grouped_if_condition(
            command,
            then_span,
            context.source,
            context.source_map(),
            context.options,
            context.facts,
        )
    {
        return IfLayoutStyle::RawGroupedCondition { raw_condition };
    }

    if if_condition_starts_after_keyword(
        command,
        then_span,
        context.source,
        context.source_map(),
        context.options,
        context.facts,
    ) || if_condition_has_explicit_statement_break(
        command,
        then_span,
        context.source,
        context.source_map(),
        context.facts,
    ) {
        return IfLayoutStyle::SplitCondition;
    }

    let can_inline_then = no_elifs
        && can_inline_body_with_upper_bound(
            context,
            &command.then_branch,
            command.span,
            Some(fi_upper_bound),
        );

    if no_elifs && command.else_branch.is_none() && can_inline_then {
        return IfLayoutStyle::Inline(InlineIfLayout::Then);
    }

    if can_inline_then && let Some(else_branch) = &command.else_branch {
        let can_inline_else = can_inline_body_with_upper_bound(
            context,
            else_branch,
            command.span,
            Some(fi_upper_bound),
        );
        if can_inline_else {
            return IfLayoutStyle::Inline(InlineIfLayout::ThenElse);
        }
        if !context.options.compact_layout() {
            return IfLayoutStyle::Inline(InlineIfLayout::ThenMultilineElse);
        }
    }

    if no_elifs
        && command.else_branch.is_none()
        && then_branch_starts_with_inline_if(context, command, then_span, fi_span)
    {
        return IfLayoutStyle::Inline(InlineIfLayout::ThenNestedIf);
    }

    if can_inline_if_chain(context, command, fi_span) {
        return IfLayoutStyle::Inline(InlineIfLayout::Chain);
    }

    if context.options.compact_layout() {
        IfLayoutStyle::Expanded(ExpandedIfLayout::Compact)
    } else {
        IfLayoutStyle::Expanded(ExpandedIfLayout::Multiline {
            inline_else_close: command
                .else_branch
                .as_ref()
                .is_some_and(|body| can_inline_else_branch_close(context, command, body, fi_span)),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CaseLayoutPlan {
    pub(super) style: CaseLayoutStyle,
    pub(super) body_fallback_upper_bound: usize,
    pub(super) esac_span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CaseLayoutStyle {
    Inline,
    Compact,
    Multiline {
        header_item_count: usize,
        blank_line_after_in: bool,
        close: CaseCloseLayout,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CaseCloseLayout {
    SameLine,
    NextLine { blank_before: bool },
    SuffixComments(Vec<BranchPrefixComment>),
}

impl CaseLayoutPlan {
    pub(super) fn for_command(command: &CaseCommand, context: RenderContext<'_, '_>) -> Self {
        let case_facts = context.facts.case_command(command);
        let body_fallback_upper_bound = case_facts.body_fallback_upper_bound();
        let esac_span = case_facts.esac_span();
        let style = if !context.options.compact_layout()
            && case_command_was_inline_in_source(command, context.source)
            && case_can_format_inline(command, context)
        {
            CaseLayoutStyle::Inline
        } else if context.options.compact_layout() {
            CaseLayoutStyle::Compact
        } else {
            CaseLayoutStyle::Multiline {
                header_item_count: case_header_item_count(command, context),
                blank_line_after_in: case_facts.has_blank_line_after_in(),
                close: case_close_layout(command, context),
            }
        };

        Self {
            style,
            body_fallback_upper_bound,
            esac_span,
        }
    }
}

pub(super) fn case_can_format_inline(
    command: &CaseCommand,
    context: RenderContext<'_, '_>,
) -> bool {
    command.cases.iter().all(|item| {
        item.body.is_empty()
            || item.body.len() == 1
                && (context.facts.case_item_was_inline_in_source(item)
                    || case_item_pattern_body_terminator_was_inline_in_source(item, context.source)
                    || case_item_body_was_inline_without_terminator(item))
                && !context
                    .facts
                    .sequence(&item.body, Some(command.span.end.offset))
                    .has_comments()
    })
}

fn case_header_item_count(command: &CaseCommand, context: RenderContext<'_, '_>) -> usize {
    let mut item_count = 0;
    for item in &command.cases {
        if !case_item_pattern_starts_on_case_header(command, item)
            || !context.facts.case_item(item).prefix_comments().is_empty()
        {
            break;
        }
        item_count += 1;
    }
    item_count
}

fn case_close_layout(command: &CaseCommand, context: RenderContext<'_, '_>) -> CaseCloseLayout {
    let case_facts = context.facts.case_command(command);
    let case_suffix_comments = case_facts.suffix_comments_before_esac().to_vec();
    if !case_suffix_comments.is_empty() {
        return CaseCloseLayout::SuffixComments(case_suffix_comments);
    }

    if case_close_shares_line_with_last_item(command, case_facts.esac_span(), context.source) {
        CaseCloseLayout::SameLine
    } else {
        CaseCloseLayout::NextLine {
            blank_before: case_facts.has_blank_line_before_esac(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::ShellFormatOptions;
    use shuck_parser::parser::Parser;

    fn parse_with_facts<'source>(
        source: &'source str,
        options: &ShellFormatOptions,
    ) -> (File, ResolvedShellFormatOptions, FormatterFacts<'source>) {
        let resolved = options.resolve(source, None);
        let file = Parser::with_dialect(source, resolved.dialect())
            .parse()
            .unwrap()
            .file;
        let facts = FormatterFacts::build(source, &file, &resolved);
        (file, resolved, facts)
    }

    fn first_if_command(file: &File) -> &IfCommand {
        let Command::Compound(CompoundCommand::If(command)) = &file.body[0].command else {
            panic!("expected first statement to be if command");
        };
        command
    }

    fn first_case_command(file: &File) -> &CaseCommand {
        let Command::Compound(CompoundCommand::Case(command)) = &file.body[0].command else {
            panic!("expected first statement to be case command");
        };
        command
    }

    #[test]
    fn plans_inline_then_if() {
        let source = "if true; then echo yes; fi\n";
        let (file, resolved, facts) = parse_with_facts(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let plan = IfLayoutPlan::then_fi(first_if_command(&file), context);

        assert!(matches!(
            plan.style,
            IfLayoutStyle::Inline(InlineIfLayout::Then)
        ));
    }

    #[test]
    fn plans_split_condition_if() {
        let source = "if\n  true\nthen\n  echo yes\nfi\n";
        let (file, resolved, facts) = parse_with_facts(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let plan = IfLayoutPlan::then_fi(first_if_command(&file), context);

        assert_eq!(plan.style, IfLayoutStyle::SplitCondition);
    }

    #[test]
    fn plans_inline_source_case_as_inline() {
        let source = "case $x in a) echo a ;; esac\n";
        let (file, resolved, facts) = parse_with_facts(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let plan = CaseLayoutPlan::for_command(first_case_command(&file), context);

        assert_eq!(plan.style, CaseLayoutStyle::Inline);
    }

    #[test]
    fn plans_header_line_case_items_for_multiline_case() {
        let source = "case $x in a) echo a ;;\nb) echo b ;;\nesac\n";
        let (file, resolved, facts) = parse_with_facts(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let plan = CaseLayoutPlan::for_command(first_case_command(&file), context);

        let CaseLayoutStyle::Multiline {
            header_item_count,
            blank_line_after_in,
            close,
        } = plan.style
        else {
            panic!("expected multiline case plan");
        };
        assert_eq!(header_item_count, 1);
        assert!(!blank_line_after_in);
        assert_eq!(
            close,
            CaseCloseLayout::NextLine {
                blank_before: false
            }
        );
    }

    #[test]
    fn plans_compact_case_when_never_split() {
        let source = "case $x in\na) echo a ;;\nesac\n";
        let options = ShellFormatOptions::default().with_never_split(true);
        let (file, resolved, facts) = parse_with_facts(source, &options);
        let context = RenderContext::new(source, &resolved, &facts);
        let plan = CaseLayoutPlan::for_command(first_case_command(&file), context);

        assert_eq!(plan.style, CaseLayoutStyle::Compact);
    }
}
