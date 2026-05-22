mod body_site;
mod case_layout;
mod compound_body;
mod conditions;
mod function_body;
mod if_case;
mod shape;

pub(crate) use body_site::CompoundBodySite;
pub(crate) use case_layout::{
    case_item_body_can_share_terminator, case_item_body_terminator_was_inline_in_source,
    case_item_body_was_inline_without_terminator, case_item_close_paren_shares_line_with_body,
    case_item_pattern_body_terminator_was_inline_in_source,
    case_item_pattern_close_paren_on_own_line, case_item_single_body_stmt_can_inline,
    case_item_started_inline_without_terminator, case_prefix_comment_uses_body_indent,
    comment_looks_like_disabled_case_pattern, trim_trailing_pattern_line_continuation,
};
pub(crate) use compound_body::{CompoundBodyLayout, CompoundBodyPlan, DoDoneBodyLayout};
pub(crate) use conditions::{
    condition_keyword_on_previous_non_empty_line, elif_condition_has_explicit_statement_break,
    loop_condition_starts_after_keyword, stmt_sequence_renders_with_subshell_open,
};
pub(crate) use function_body::{
    FunctionBodyGroupLayout, FunctionBodyLayout, FunctionBodyPlan, FunctionSubshellLayout,
};
pub(crate) use if_case::{
    CaseCloseLayout, CaseLayoutPlan, CaseLayoutStyle, ExpandedIfLayout, IfLayoutPlan,
    IfLayoutStyle, InlineIfLayout, case_can_format_inline,
};
pub(crate) use shape::{
    can_format_multiline_subshell_inline, can_inline_group, can_inline_source_line_subshell,
    group_has_inline_source_shape,
};

#[cfg(test)]
mod layout_plan_tests {
    use shuck_ast::{
        Command, CompoundCommand, File, FunctionDef, RepeatCommand, Stmt, WhileCommand,
    };

    use super::*;
    use crate::context::RenderContext;
    use crate::options::{ShellDialect, ShellFormatOptions};

    fn context_for<'source>(
        source: &'source str,
        options: &ShellFormatOptions,
    ) -> (
        File,
        crate::options::ResolvedShellFormatOptions,
        crate::facts::FormatterFacts<'source>,
    ) {
        if_case::parse_with_facts(source, options)
    }

    fn first_while_command(file: &File) -> &WhileCommand {
        let Command::Compound(CompoundCommand::While(command)) = &file.body[0].command else {
            panic!("expected first statement to be while command");
        };
        command
    }

    fn first_repeat_command(file: &File) -> &RepeatCommand {
        let Command::Compound(CompoundCommand::Repeat(command)) = &file.body[0].command else {
            panic!("expected first statement to be repeat command");
        };
        command
    }

    fn first_function(file: &File) -> &FunctionDef {
        let Command::Function(function) = &file.body[0].command else {
            panic!("expected first statement to be function definition");
        };
        function
    }

    fn function_body(function: &FunctionDef) -> (&Stmt, usize) {
        (function.body.as_ref(), function.span.end.offset)
    }

    #[test]
    fn layout_plan_plans_inline_then_if() {
        let source = "if true; then echo yes; fi\n";
        let (file, resolved, facts) = context_for(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let plan = IfLayoutPlan::then_fi(if_case::first_if_command(&file), context);

        assert!(matches!(
            plan.style,
            IfLayoutStyle::Inline(InlineIfLayout::Then)
        ));
    }

    #[test]
    fn layout_plan_plans_split_condition_if() {
        let source = "if\n  true\nthen\n  echo yes\nfi\n";
        let (file, resolved, facts) = context_for(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let plan = IfLayoutPlan::then_fi(if_case::first_if_command(&file), context);

        assert_eq!(plan.style, IfLayoutStyle::SplitCondition);
    }

    #[test]
    fn layout_plan_plans_inline_source_case_as_inline() {
        let source = "case $x in a) echo a ;; esac\n";
        let (file, resolved, facts) = context_for(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let plan = CaseLayoutPlan::for_command(if_case::first_case_command(&file), context);

        assert_eq!(plan.style, CaseLayoutStyle::Inline);
    }

    #[test]
    fn layout_plan_plans_header_line_case_items_for_multiline_case() {
        let source = "case $x in a) echo a ;;\nb) echo b ;;\nesac\n";
        let (file, resolved, facts) = context_for(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let plan = CaseLayoutPlan::for_command(if_case::first_case_command(&file), context);

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
    fn layout_plan_plans_inline_do_done_body() {
        let source = "while ok; do echo hi; done\n";
        let (file, resolved, facts) = context_for(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let command = first_while_command(&file);
        let site = CompoundBodySite::while_command(
            command,
            context.source_map(),
            facts.compound_close_span_for_span(command.span),
        );
        let plan = CompoundBodyPlan::do_done(site, context);

        assert_eq!(
            plan.layout(),
            CompoundBodyLayout::DoDone(DoDoneBodyLayout::Inline)
        );
    }

    #[test]
    fn layout_plan_plans_multiline_do_done_body_with_open_suffix() {
        let source = "while ok; do # note\n  echo hi\ndone\n";
        let (file, resolved, facts) = context_for(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let command = first_while_command(&file);
        let site = CompoundBodySite::while_command(
            command,
            context.source_map(),
            facts.compound_close_span_for_span(command.span),
        );
        let plan = CompoundBodyPlan::do_done(site, context);

        assert_eq!(
            plan.layout(),
            CompoundBodyLayout::DoDone(DoDoneBodyLayout::Multiline { open: "; do" })
        );
        assert!(plan.open_suffix_span().is_some());
    }

    #[test]
    fn layout_plan_plans_split_do_done_with_plain_do_opener() {
        let source = "while\n  ok\ndo\n  echo hi\ndone\n";
        let (file, resolved, facts) = context_for(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let command = first_while_command(&file);
        let site = CompoundBodySite::while_command(
            command,
            context.source_map(),
            facts.compound_close_span_for_span(command.span),
        );
        let plan = CompoundBodyPlan::split_do_done(site, context);

        assert_eq!(
            plan.layout(),
            CompoundBodyLayout::DoDone(DoDoneBodyLayout::Multiline { open: "do" })
        );
    }

    #[test]
    fn layout_plan_plans_direct_repeat_body() {
        let source = "repeat 3 echo hi\n";
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Zsh);
        let (file, resolved, facts) = context_for(source, &options);
        let context = RenderContext::new(source, &resolved, &facts);
        let command = first_repeat_command(&file);
        let site = CompoundBodySite::repeat_command(
            command,
            context.source_map(),
            facts.compound_close_span_for_span(command.span),
        );
        let plan = CompoundBodyPlan::loop_body(site, " ", context);

        assert_eq!(plan.layout(), CompoundBodyLayout::DirectInline);
    }

    #[test]
    fn layout_plan_plans_legacy_inline_do_brace_group_separator() {
        let source = "for item in $items; do {\n  case \"$item\" in\n  a)\n    echo a\n    ;;\n  esac\n} done\n";
        let (file, resolved, facts) = context_for(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let Command::Compound(CompoundCommand::For(command)) = &file.body[0].command else {
            panic!("expected first statement to be for command");
        };
        let site = CompoundBodySite::for_command(
            command,
            context.source_map(),
            facts.compound_close_span_for_span(command.span),
        );
        let plan = CompoundBodyPlan::do_done(site, context);

        assert_eq!(
            plan.layout(),
            CompoundBodyLayout::DoDone(DoDoneBodyLayout::LegacyInline {
                close_separator: " "
            })
        );
    }

    #[test]
    fn layout_plan_plans_legacy_inline_do_if_separator() {
        let source = "while read -r line; do {\n  if ok; then\n    :\n  fi\n} done <file\n";
        let (file, resolved, facts) = context_for(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let command = first_while_command(&file);
        let site = CompoundBodySite::while_command(
            command,
            context.source_map(),
            facts.compound_close_span_for_span(command.span),
        );
        let plan = CompoundBodyPlan::do_done(site, context);

        assert_eq!(
            plan.layout(),
            CompoundBodyLayout::DoDone(DoDoneBodyLayout::LegacyInline {
                close_separator: "; "
            })
        );
    }

    #[test]
    fn layout_plan_plans_inline_function_brace_body() {
        let source = "f() { echo hi; }\n";
        let (file, resolved, facts) = context_for(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let (body, upper_bound) = function_body(first_function(&file));
        let plan = FunctionBodyPlan::for_body(body, upper_bound, false, context);

        assert!(matches!(
            plan.layout(),
            FunctionBodyLayout::BraceGroup {
                layout: FunctionBodyGroupLayout::Inline,
                ..
            }
        ));
    }

    #[test]
    fn layout_plan_plans_multiline_function_brace_body() {
        let source = "f() {\n  echo hi\n  echo bye\n}\n";
        let (file, resolved, facts) = context_for(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let (body, upper_bound) = function_body(first_function(&file));
        let plan = FunctionBodyPlan::for_body(body, upper_bound, false, context);

        assert!(matches!(
            plan.layout(),
            FunctionBodyLayout::BraceGroup {
                layout: FunctionBodyGroupLayout::Multiline,
                ..
            }
        ));
    }

    #[test]
    fn layout_plan_plans_function_header_comment_body() {
        let source = "f() # header\n{\n  echo hi\n}\n";
        let (file, resolved, facts) = context_for(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let (body, upper_bound) = function_body(first_function(&file));
        let plan = FunctionBodyPlan::for_body(body, upper_bound, true, context);

        assert!(matches!(
            plan.layout(),
            FunctionBodyLayout::BraceGroup {
                layout: FunctionBodyGroupLayout::HeaderComment,
                ..
            }
        ));
    }

    #[test]
    fn layout_plan_plans_inline_function_subshell_body() {
        let source = "f() (echo hi)\n";
        let (file, resolved, facts) = context_for(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let (body, upper_bound) = function_body(first_function(&file));
        let plan = FunctionBodyPlan::for_body(body, upper_bound, false, context);

        assert!(matches!(
            plan.layout(),
            FunctionBodyLayout::Subshell {
                layout: FunctionSubshellLayout::Inline { .. },
                ..
            }
        ));
    }

    #[test]
    fn layout_plan_plans_multiline_function_subshell_body() {
        let source = "f() (\n  echo hi\n  echo bye\n)\n";
        let (file, resolved, facts) = context_for(source, &ShellFormatOptions::default());
        let context = RenderContext::new(source, &resolved, &facts);
        let (body, upper_bound) = function_body(first_function(&file));
        let plan = FunctionBodyPlan::for_body(body, upper_bound, false, context);

        assert!(matches!(
            plan.layout(),
            FunctionBodyLayout::Subshell {
                layout: FunctionSubshellLayout::Multiline,
                ..
            }
        ));
    }
}
