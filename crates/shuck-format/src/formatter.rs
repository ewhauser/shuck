use crate::format_element::{Document, FormatElement};
use crate::printer::{Printed, Printer, PrinterOptions};

pub type FormatResult<T> = Result<T, FormatError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError {
    message: String,
}

impl FormatError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FormatError {}

pub trait FormatContext {
    type Options: FormatOptions;

    fn options(&self) -> &Self::Options;
}

pub trait FormatOptions {
    fn as_print_options(&self) -> PrinterOptions;
}

pub trait Format<Context> {
    fn fmt(&self, formatter: &mut Formatter<Context>) -> FormatResult<()>;
}

impl<Context, T> Format<Context> for &T
where
    T: Format<Context>,
{
    fn fmt(&self, formatter: &mut Formatter<Context>) -> FormatResult<()> {
        (*self).fmt(formatter)
    }
}

impl<Context, T> Format<Context> for Option<T>
where
    T: Format<Context>,
{
    fn fmt(&self, formatter: &mut Formatter<Context>) -> FormatResult<()> {
        if let Some(value) = self {
            value.fmt(formatter)?;
        }
        Ok(())
    }
}

impl<Context> Format<Context> for Document {
    fn fmt(&self, formatter: &mut Formatter<Context>) -> FormatResult<()> {
        formatter.write_document(self.clone());
        Ok(())
    }
}

impl<Context> Format<Context> for FormatElement {
    fn fmt(&self, formatter: &mut Formatter<Context>) -> FormatResult<()> {
        formatter.write_document(Document::from_element(self.clone()));
        Ok(())
    }
}

pub struct Formatter<Context> {
    context: Context,
    document: Document,
}

impl<Context> Formatter<Context> {
    #[must_use]
    pub fn new(context: Context) -> Self {
        Self {
            context,
            document: Document::new(),
        }
    }

    #[must_use]
    pub fn context(&self) -> &Context {
        &self.context
    }

    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.context
    }

    pub fn write_document(&mut self, document: Document) {
        self.document.extend(document);
    }

    #[must_use]
    pub fn finish(self) -> Formatted<Context> {
        Formatted {
            context: self.context,
            document: self.document,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Formatted<Context> {
    context: Context,
    document: Document,
}

impl<Context> Formatted<Context>
where
    Context: FormatContext,
{
    pub fn print(&self) -> FormatResult<Printed> {
        let printer = Printer::new(self.context.options().as_print_options());
        printer
            .print(&self.document)
            .map_err(|err| FormatError::new(err.to_string()))
    }

    #[must_use]
    pub fn context(&self) -> &Context {
        &self.context
    }

    #[must_use]
    pub fn document(&self) -> &Document {
        &self.document
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimpleFormatOptions {
    pub printer_options: PrinterOptions,
}

impl FormatOptions for SimpleFormatOptions {
    fn as_print_options(&self) -> PrinterOptions {
        self.printer_options
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimpleFormatContext {
    pub options: SimpleFormatOptions,
}

impl SimpleFormatContext {
    #[must_use]
    pub fn new(options: SimpleFormatOptions) -> Self {
        Self { options }
    }
}

impl FormatContext for SimpleFormatContext {
    type Options = SimpleFormatOptions;

    fn options(&self) -> &Self::Options {
        &self.options
    }
}
