use shuck_ast::Stmt;

use crate::context::RenderContext;

use super::body_site::CompoundBodySite;
use super::shape::{
    can_format_multiline_subshell_inline, can_inline_group, can_inline_source_line_subshell,
    group_has_inline_source_shape,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct FunctionBodyPlan<'a> {
    body: &'a Stmt,
    layout: FunctionBodyLayout<'a>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum FunctionBodyLayout<'a> {
    FallbackStmt,
    BraceGroup {
        site: CompoundBodySite<'a>,
        layout: FunctionBodyGroupLayout,
    },
    Subshell {
        site: CompoundBodySite<'a>,
        layout: FunctionSubshellLayout,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionBodyGroupLayout {
    Inline,
    Multiline,
    HeaderComment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionSubshellLayout {
    Inline { source_line: bool },
    MultilineInline,
    Multiline,
}

impl<'a> FunctionBodyPlan<'a> {
    pub(crate) fn for_body(
        body: &'a Stmt,
        upper_bound: usize,
        has_header_comment: bool,
        context: RenderContext<'_, '_>,
    ) -> Self {
        let Some(site) = CompoundBodySite::function_group_body(body, upper_bound) else {
            return Self {
                body,
                layout: FunctionBodyLayout::FallbackStmt,
            };
        };

        let layout = match site.group_open_char() {
            Some('{') => FunctionBodyLayout::BraceGroup {
                site,
                layout: brace_group_layout(site, has_header_comment, context),
            },
            Some('(') => FunctionBodyLayout::Subshell {
                site,
                layout: subshell_layout(site, context),
            },
            _ => unreachable!("function body group uses brace or subshell syntax"),
        };
        Self { body, layout }
    }

    pub(crate) fn body(self) -> &'a Stmt {
        self.body
    }

    pub(crate) fn layout(self) -> FunctionBodyLayout<'a> {
        self.layout
    }
}

fn brace_group_layout(
    site: CompoundBodySite<'_>,
    has_header_comment: bool,
    context: RenderContext<'_, '_>,
) -> FunctionBodyGroupLayout {
    if has_header_comment {
        return FunctionBodyGroupLayout::HeaderComment;
    }

    let body = site.body();
    let upper_bound = site.bounds().render_limit();
    if !context.options.function_next_line()
        && context
            .facts
            .sequence(body, upper_bound)
            .group_open_suffix_span()
            .is_none()
        && group_has_inline_source_shape(context, body, '{')
        && can_inline_group(context, body, '{')
    {
        FunctionBodyGroupLayout::Inline
    } else {
        FunctionBodyGroupLayout::Multiline
    }
}

fn subshell_layout(
    site: CompoundBodySite<'_>,
    context: RenderContext<'_, '_>,
) -> FunctionSubshellLayout {
    let body = site.body();
    let upper_bound = site.bounds().render_limit();
    let sequence = context.facts.sequence(body, upper_bound);
    let group_inline =
        group_has_inline_source_shape(context, body, '(') && can_inline_group(context, body, '(');
    let source_line_inline = can_inline_source_line_subshell(context, body, upper_bound);

    if !context.options.function_next_line()
        && sequence.group_open_suffix_span().is_none()
        && (group_inline || source_line_inline)
    {
        return FunctionSubshellLayout::Inline {
            source_line: source_line_inline && !group_inline,
        };
    }

    if can_format_multiline_subshell_inline(context, body, upper_bound) {
        FunctionSubshellLayout::MultilineInline
    } else {
        FunctionSubshellLayout::Multiline
    }
}
