use shuck_ast::{Span, StmtSeq};

use crate::context::RenderContext;

use super::body_site::{CompoundBodyOpen, CompoundBodySite};
use super::shape::{
    body_starts_with_inline_do_brace_group, body_starts_with_inline_do_if,
    can_inline_body_with_upper_bound, inline_do_brace_group_done_separator,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct CompoundBodyPlan<'a> {
    site: CompoundBodySite<'a>,
    layout: CompoundBodyLayout,
    open_suffix_span: Option<Span>,
    preserve_open_blank: bool,
    preserve_close_blank: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompoundBodyLayout {
    DoDone(DoDoneBodyLayout),
    DirectInline,
    BraceGroup { prefix: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DoDoneBodyLayout {
    Inline,
    LegacyInline { close_separator: &'static str },
    Multiline { open: &'static str },
}

impl<'a> CompoundBodyPlan<'a> {
    pub(crate) fn loop_body(
        site: CompoundBodySite<'a>,
        brace_prefix: &'static str,
        context: RenderContext<'_, '_>,
    ) -> Self {
        match site.open() {
            CompoundBodyOpen::Keyword("do") => Self::do_done(site, context),
            CompoundBodyOpen::Direct => {
                Self::from_layout(site, CompoundBodyLayout::DirectInline, context)
            }
            CompoundBodyOpen::Group('{') => Self::brace_group(site, brace_prefix, context),
            _ => unreachable!("compound body uses do, direct, or brace syntax"),
        }
    }

    pub(crate) fn do_done(site: CompoundBodySite<'a>, context: RenderContext<'_, '_>) -> Self {
        let body = site.body();
        let body_upper_bound = site.bounds().render_end();
        let open_suffix_span = context
            .facts
            .sequence(body, Some(body_upper_bound))
            .group_open_suffix_span();
        let layout = if open_suffix_span.is_none() {
            if can_inline_body_with_upper_bound(
                context,
                body,
                site.enclosing_span(),
                Some(body_upper_bound),
            ) {
                DoDoneBodyLayout::Inline
            } else if body_starts_with_inline_do_brace_group(context, body) {
                DoDoneBodyLayout::LegacyInline {
                    close_separator: inline_do_brace_group_done_separator(
                        context,
                        body,
                        site.enclosing_span(),
                    ),
                }
            } else if body_starts_with_inline_do_if(context, body) {
                DoDoneBodyLayout::LegacyInline {
                    close_separator: "; ",
                }
            } else {
                DoDoneBodyLayout::Multiline { open: "; do" }
            }
        } else {
            DoDoneBodyLayout::Multiline { open: "; do" }
        };
        Self::from_layout(site, CompoundBodyLayout::DoDone(layout), context)
    }

    pub(crate) fn split_do_done(
        site: CompoundBodySite<'a>,
        context: RenderContext<'_, '_>,
    ) -> Self {
        Self::from_layout(
            site,
            CompoundBodyLayout::DoDone(DoDoneBodyLayout::Multiline { open: "do" }),
            context,
        )
    }

    pub(crate) fn brace_group(
        site: CompoundBodySite<'a>,
        prefix: &'static str,
        context: RenderContext<'_, '_>,
    ) -> Self {
        Self::from_layout(site, CompoundBodyLayout::BraceGroup { prefix }, context)
    }

    fn from_layout(
        site: CompoundBodySite<'a>,
        layout: CompoundBodyLayout,
        context: RenderContext<'_, '_>,
    ) -> Self {
        let body = site.body();
        let upper_bound = site.bounds().render_limit();
        let sequence = context.facts.sequence(body, upper_bound);
        Self {
            site,
            layout,
            open_suffix_span: sequence.group_open_suffix_span(),
            preserve_open_blank: sequence.has_blank_line_after_open(),
            preserve_close_blank: sequence.has_blank_line_before_close(),
        }
    }

    pub(crate) fn body(self) -> &'a StmtSeq {
        self.site.body()
    }

    pub(crate) fn body_upper_bound(self) -> usize {
        self.site.bounds().render_end()
    }

    pub(crate) fn upper_bound(self) -> Option<usize> {
        self.site.bounds().render_limit()
    }

    pub(crate) fn close_span(self) -> Option<Span> {
        self.site.close_span()
    }

    pub(crate) fn layout(self) -> CompoundBodyLayout {
        self.layout
    }

    pub(crate) fn open_suffix_span(self) -> Option<Span> {
        self.open_suffix_span
    }

    pub(crate) fn preserve_open_blank(self) -> bool {
        self.preserve_open_blank
    }

    pub(crate) fn preserve_close_blank(self) -> bool {
        self.preserve_close_blank
    }
}
