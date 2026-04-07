mod format_element;
mod formatter;
mod macros;
pub mod prelude;
mod printer;

pub use crate::format_element::{
    Document, FormatElement, LineMode, best_fit, group, hard_line_break, indent, soft_line_break,
    soft_line_break_or_space, space, text, verbatim,
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
}
