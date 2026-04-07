use crate::format_element::{Document, FormatElement, LineMode};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IndentStyle {
    #[default]
    Tab,
    Space,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

impl LineEnding {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrinterOptions {
    pub indent_style: IndentStyle,
    pub indent_width: u8,
    pub line_width: u16,
    pub line_ending: LineEnding,
}

impl Default for PrinterOptions {
    fn default() -> Self {
        Self {
            indent_style: IndentStyle::Tab,
            indent_width: 4,
            line_width: 80,
            line_ending: LineEnding::Lf,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printed {
    code: String,
}

impl Printed {
    #[must_use]
    pub fn as_code(&self) -> &str {
        &self.code
    }

    #[must_use]
    pub fn into_code(self) -> String {
        self.code
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintError {
    message: String,
}

impl std::fmt::Display for PrintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PrintError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrintMode {
    Flat,
    Expanded,
}

#[derive(Debug, Clone)]
pub struct Printer {
    options: PrinterOptions,
}

impl Printer {
    #[must_use]
    pub fn new(options: PrinterOptions) -> Self {
        Self { options }
    }

    pub fn print(&self, document: &Document) -> Result<Printed, PrintError> {
        self.print_with_capacity(document, 0)
    }

    pub fn print_with_capacity(
        &self,
        document: &Document,
        output_capacity: usize,
    ) -> Result<Printed, PrintError> {
        let mut output = String::with_capacity(output_capacity);
        let mut state = PrinterState::new(self.options);
        self.print_document(document, PrintMode::Expanded, &mut state, &mut output)?;
        Ok(Printed { code: output })
    }

    fn print_document(
        &self,
        document: &Document,
        mode: PrintMode,
        state: &mut PrinterState,
        output: &mut String,
    ) -> Result<(), PrintError> {
        for element in document.as_slice() {
            match element {
                FormatElement::Text(text) => state.push_text(text, output),
                FormatElement::Space => state.push_text(" ", output),
                FormatElement::Line(line_mode) => match (mode, line_mode) {
                    (PrintMode::Flat, LineMode::Soft) => {}
                    (PrintMode::Flat, LineMode::SoftOrSpace) => state.push_text(" ", output),
                    (_, LineMode::Hard) | (PrintMode::Expanded, _) => {
                        state.push_newline(output);
                    }
                },
                FormatElement::Indent(inner) => {
                    state.indent_level += 1;
                    self.print_document(inner, mode, state, output)?;
                    state.indent_level = state.indent_level.saturating_sub(1);
                }
                FormatElement::Group(inner) => {
                    let fits_flat = self.fits(inner, state)?;
                    let next_mode = if fits_flat {
                        PrintMode::Flat
                    } else {
                        PrintMode::Expanded
                    };
                    self.print_document(inner, next_mode, state, output)?;
                }
                FormatElement::BestFit { flat, expanded } => {
                    if self.fits(flat, state)? {
                        self.print_document(flat, PrintMode::Flat, state, output)?;
                    } else {
                        self.print_document(expanded, PrintMode::Expanded, state, output)?;
                    }
                }
                FormatElement::Verbatim(text) => state.push_verbatim(text, output),
            }
        }

        Ok(())
    }

    fn fits(&self, document: &Document, state: &PrinterState) -> Result<bool, PrintError> {
        let mut width = state.column;
        self.measure_document(document, &mut width)?;
        Ok(width <= usize::from(self.options.line_width))
    }

    fn measure_document(&self, document: &Document, width: &mut usize) -> Result<(), PrintError> {
        for element in document.as_slice() {
            match element {
                FormatElement::Text(text) => *width += text.chars().count(),
                FormatElement::Space => *width += 1,
                FormatElement::Line(LineMode::Hard) => return Ok(()),
                FormatElement::Line(LineMode::Soft) => {}
                FormatElement::Line(LineMode::SoftOrSpace) => *width += 1,
                FormatElement::Indent(inner) | FormatElement::Group(inner) => {
                    self.measure_document(inner, width)?;
                }
                FormatElement::BestFit { flat, .. } => {
                    self.measure_document(flat, width)?;
                }
                FormatElement::Verbatim(text) => {
                    let line = text.rsplit('\n').next().unwrap_or(text);
                    *width += line.chars().count();
                }
            }

            if *width > usize::from(self.options.line_width) {
                return Ok(());
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct PrinterState {
    options: PrinterOptions,
    indent_level: usize,
    column: usize,
    line_has_content: bool,
}

impl PrinterState {
    fn new(options: PrinterOptions) -> Self {
        Self {
            options,
            indent_level: 0,
            column: 0,
            line_has_content: false,
        }
    }

    fn push_text(&mut self, text: &str, output: &mut String) {
        if !self.line_has_content {
            self.push_indent(output);
        }
        output.push_str(text);
        self.column += text.chars().count();
        self.line_has_content = true;
    }

    fn push_newline(&mut self, output: &mut String) {
        output.push_str(self.options.line_ending.as_str());
        self.column = 0;
        self.line_has_content = false;
    }

    fn push_indent(&mut self, output: &mut String) {
        if self.line_has_content || self.indent_level == 0 {
            return;
        }

        let indent = match self.options.indent_style {
            IndentStyle::Tab => "\t".repeat(self.indent_level),
            IndentStyle::Space => {
                " ".repeat(self.indent_level * usize::from(self.options.indent_width))
            }
        };

        output.push_str(&indent);
        self.column += indent.chars().count();
        self.line_has_content = true;
    }

    fn push_verbatim(&mut self, text: &str, output: &mut String) {
        if !self.line_has_content {
            self.push_indent(output);
        }

        output.push_str(text);
        if let Some(last_line) = text.rsplit('\n').next() {
            self.column = last_line.chars().count();
        } else {
            self.column += text.chars().count();
        }
        self.line_has_content = !text.ends_with('\n');
    }
}
