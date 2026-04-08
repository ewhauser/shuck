mod buffer;
mod format_element;
mod formatter;
mod macros;
pub mod prelude;
mod printer;

pub use crate::buffer::{Buffer, VecBuffer};
pub use crate::format_element::{
    Document, FormatElement, LineMode, best_fit, group, hard_line_break, indent, soft_line_break,
    soft_line_break_or_space, space, text, token, verbatim,
};
pub use crate::formatter::{
    Format, FormatContext, FormatError, FormatOptions, FormatResult, Formatted, Formatter,
    SimpleFormatContext, SimpleFormatOptions,
};
pub use crate::printer::{IndentStyle, LineEnding, PrintError, Printed, Printer, PrinterOptions};

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use crate::{SimpleFormatContext, SimpleFormatOptions, format};

    #[test]
    fn prints_indented_hard_lines() {
        let context = SimpleFormatContext::new(SimpleFormatOptions::default());
        let doc = Document::from_elements(vec![
            text("if"),
            hard_line_break(),
            indent(Document::from_elements(vec![
                text("echo hi"),
                hard_line_break(),
                text("echo bye"),
            ])),
        ]);

        let printed = format!(context, [doc]).unwrap().print().unwrap();
        assert_eq!(printed.as_code(), "if\n\techo hi\n\techo bye");
    }

    #[test]
    fn group_flattens_when_content_fits() {
        let context = SimpleFormatContext::new(SimpleFormatOptions::default());
        let doc = Document::from_element(group(Document::from_elements(vec![
            text("echo"),
            soft_line_break_or_space(),
            text("hello"),
        ])));

        let printed = format!(context, [doc]).unwrap().print().unwrap();
        assert_eq!(printed.as_code(), "echo hello");
    }

    #[test]
    fn group_expands_when_content_overflows() {
        let mut options = SimpleFormatOptions::default();
        options.printer_options.line_width = 8;
        let context = SimpleFormatContext::new(options);
        let doc = Document::from_element(group(Document::from_elements(vec![
            text("echo"),
            soft_line_break_or_space(),
            text("long-value"),
        ])));

        let printed = format!(context, [doc]).unwrap().print().unwrap();
        assert_eq!(printed.as_code(), "echo\nlong-value");
    }

    #[test]
    fn best_fit_chooses_expanded_variant() {
        let mut options = SimpleFormatOptions::default();
        options.printer_options.line_width = 6;
        let context = SimpleFormatContext::new(options);
        let doc = Document::from_element(best_fit(
            Document::from_elements(vec![text("alpha"), space(), text("beta")]),
            Document::from_elements(vec![text("alpha"), hard_line_break(), text("beta")]),
        ));

        let printed = format!(context, [doc]).unwrap().print().unwrap();
        assert_eq!(printed.as_code(), "alpha\nbeta");
    }

    #[test]
    fn group_expands_for_wide_unicode_text() {
        let mut options = SimpleFormatOptions::default();
        options.printer_options.line_width = 3;
        let context = SimpleFormatContext::new(options);
        let doc = Document::from_element(group(Document::from_elements(vec![
            text("a"),
            soft_line_break_or_space(),
            text("界"),
        ])));

        let printed = format!(context, [doc]).unwrap().print().unwrap();
        assert_eq!(printed.as_code(), "a\n界");
    }

    #[test]
    fn hard_lines_use_configured_crlf_endings() {
        let mut options = SimpleFormatOptions::default();
        options.printer_options.line_ending = LineEnding::CrLf;
        let context = SimpleFormatContext::new(options);
        let doc = Document::from_elements(vec![token("a"), hard_line_break(), token("b")]);

        let printed = format!(context, [doc]).unwrap().print().unwrap();
        assert_eq!(printed.as_code(), "a\r\nb");
    }

    #[test]
    fn verbatim_preserves_source_text() {
        let context = SimpleFormatContext::new(SimpleFormatOptions::default());
        let doc = Document::from_elements(vec![
            text("begin"),
            hard_line_break(),
            verbatim("  raw\ntext"),
        ]);

        let printed = format!(context, [doc]).unwrap().print().unwrap();
        assert_eq!(printed.as_code(), "begin\n  raw\ntext");
    }

    #[test]
    fn nested_groups_and_indents_expand_consistently() {
        let mut options = SimpleFormatOptions::default();
        options.printer_options.line_width = 8;
        let context = SimpleFormatContext::new(options);
        let doc = Document::from_element(group(Document::from_elements(vec![
            token("if"),
            soft_line_break_or_space(),
            token("true"),
            token(";"),
            soft_line_break(),
            indent(Document::from_elements(vec![
                token("echo"),
                soft_line_break_or_space(),
                token("long-value"),
            ])),
        ])));

        let printed = format!(context, [doc]).unwrap().print().unwrap();
        assert_eq!(printed.as_code(), "if\ntrue;\n\techo\n\tlong-value");
    }

    #[test]
    fn indents_with_spaces_when_configured() {
        let mut options = SimpleFormatOptions::default();
        options.printer_options.indent_style = IndentStyle::Space;
        options.printer_options.indent_width = 2;
        let context = SimpleFormatContext::new(options);
        let doc = Document::from_elements(vec![
            token("if"),
            hard_line_break(),
            indent(Document::from_elements(vec![
                token("echo hi"),
                hard_line_break(),
                token("echo bye"),
            ])),
        ]);

        let printed = format!(context, [doc]).unwrap().print().unwrap();
        assert_eq!(printed.as_code(), "if\n  echo hi\n  echo bye");
    }
}
