use crate::format_element::{Document, FormatElement, InternedDocument, LineMode, TextMetrics};

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
        let mut queue = PrintQueue::new(document.as_slice());
        let mut mode_stack = Vec::new();

        while let Some(element) = queue.next(&mut state, &mut mode_stack) {
            match element {
                FormatElement::Token(text) => state.push_token(text, &mut output),
                FormatElement::Text(text) => {
                    state.push_text(text.as_str(), text.metrics(), &mut output);
                }
                FormatElement::Space => state.push_token(" ", &mut output),
                FormatElement::Line(line_mode) => {
                    let mode = mode_stack.last().copied().unwrap_or(PrintMode::Expanded);
                    match (mode, line_mode) {
                        (PrintMode::Flat, LineMode::Soft) => {}
                        (PrintMode::Flat, LineMode::SoftOrSpace) => {
                            state.push_token(" ", &mut output);
                        }
                        (_, LineMode::Hard) | (PrintMode::Expanded, _) => {
                            state.push_newline(&mut output);
                        }
                    }
                }
                FormatElement::Indent(document) => {
                    queue.push(document.as_slice(), None, 1, &mut state, &mut mode_stack);
                }
                FormatElement::Group(document) => {
                    let mode = if self.fits(document.as_slice(), PrintMode::Flat, state.column) {
                        PrintMode::Flat
                    } else {
                        PrintMode::Expanded
                    };
                    queue.push(document.as_slice(), Some(mode), 0, &mut state, &mut mode_stack);
                }
                FormatElement::BestFit { flat, expanded } => {
                    let (variant, mode) =
                        if self.fits(flat.as_slice(), PrintMode::Flat, state.column) {
                            (flat.as_slice(), PrintMode::Flat)
                        } else {
                            (expanded.as_slice(), PrintMode::Expanded)
                        };
                    queue.push(variant, Some(mode), 0, &mut state, &mut mode_stack);
                }
                FormatElement::Verbatim(text) => {
                    state.push_text(text.as_str(), text.metrics(), &mut output);
                }
            }
        }

        Ok(Printed { code: output })
    }

    fn fits(&self, document: &[FormatElement], mode: PrintMode, column: usize) -> bool {
        let mut width = column;
        let mut queue = MeasureQueue::new(document, mode);
        let line_width = usize::from(self.options.line_width);

        while let Some((element, mode)) = queue.next() {
            match element {
                FormatElement::Token(text) => width += text.len(),
                FormatElement::Text(text) => {
                    if let Some(single_line_width) = text.metrics().single_line_width() {
                        width += single_line_width;
                    } else {
                        width += text.metrics().first_line_width();
                        return width <= line_width;
                    }
                }
                FormatElement::Space => width += 1,
                FormatElement::Line(line_mode) => match (mode, line_mode) {
                    (PrintMode::Flat, LineMode::Soft) => {}
                    (PrintMode::Flat, LineMode::SoftOrSpace) => width += 1,
                    (_, LineMode::Hard) | (PrintMode::Expanded, _) => return width <= line_width,
                },
                FormatElement::Indent(document) => queue.push(document, None),
                FormatElement::Group(document) => queue.push(document, Some(PrintMode::Flat)),
                FormatElement::BestFit { flat, .. } => queue.push(flat, Some(PrintMode::Flat)),
                FormatElement::Verbatim(text) => {
                    if let Some(single_line_width) = text.metrics().single_line_width() {
                        width += single_line_width;
                    } else {
                        width += text.metrics().first_line_width();
                        return width <= line_width;
                    }
                }
            }

            if width > line_width {
                return false;
            }
        }

        true
    }
}

#[derive(Debug, Clone)]
struct PrintFrame<'a> {
    elements: &'a [FormatElement],
    index: usize,
    mode: Option<PrintMode>,
    indent_delta: usize,
}

#[derive(Debug, Clone)]
struct PrintQueue<'a> {
    frames: Vec<PrintFrame<'a>>,
}

impl<'a> PrintQueue<'a> {
    fn new(elements: &'a [FormatElement]) -> Self {
        Self {
            frames: vec![PrintFrame {
                elements,
                index: 0,
                mode: None,
                indent_delta: 0,
            }],
        }
    }

    fn push(
        &mut self,
        elements: &'a [FormatElement],
        mode: Option<PrintMode>,
        indent_delta: usize,
        state: &mut PrinterState,
        mode_stack: &mut Vec<PrintMode>,
    ) {
        if indent_delta > 0 {
            state.indent_level += indent_delta;
        }
        if let Some(mode) = mode {
            mode_stack.push(mode);
        }
        self.frames.push(PrintFrame {
            elements,
            index: 0,
            mode,
            indent_delta,
        });
    }

    fn next(
        &mut self,
        state: &mut PrinterState,
        mode_stack: &mut Vec<PrintMode>,
    ) -> Option<&'a FormatElement> {
        loop {
            let frame = self.frames.last_mut()?;
            if frame.index < frame.elements.len() {
                let element = &frame.elements[frame.index];
                frame.index += 1;
                return Some(element);
            }

            let frame = self.frames.pop()?;
            state.indent_level = state.indent_level.saturating_sub(frame.indent_delta);
            if frame.mode.is_some() {
                mode_stack.pop();
            }
        }
    }
}

#[derive(Debug, Clone)]
struct MeasureFrame<'a> {
    elements: &'a [FormatElement],
    index: usize,
    mode: Option<PrintMode>,
}

#[derive(Debug, Clone)]
struct MeasureQueue<'a> {
    frames: Vec<MeasureFrame<'a>>,
    mode_stack: Vec<PrintMode>,
}

impl<'a> MeasureQueue<'a> {
    fn new(elements: &'a [FormatElement], mode: PrintMode) -> Self {
        Self {
            frames: vec![MeasureFrame {
                elements,
                index: 0,
                mode: Some(mode),
            }],
            mode_stack: vec![mode],
        }
    }

    fn push(&mut self, document: &'a InternedDocument, mode: Option<PrintMode>) {
        if let Some(mode) = mode {
            self.mode_stack.push(mode);
        }
        self.frames.push(MeasureFrame {
            elements: document.as_slice(),
            index: 0,
            mode,
        });
    }

    fn next(&mut self) -> Option<(&'a FormatElement, PrintMode)> {
        loop {
            let frame = self.frames.last_mut()?;
            if frame.index < frame.elements.len() {
                let element = &frame.elements[frame.index];
                frame.index += 1;
                let mode = self.mode_stack.last().copied().unwrap_or(PrintMode::Expanded);
                return Some((element, mode));
            }

            let frame = self.frames.pop()?;
            if frame.mode.is_some() {
                self.mode_stack.pop();
            }
        }
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

    fn push_token(&mut self, text: &str, output: &mut String) {
        if !self.line_has_content {
            self.push_indent(output);
        }
        output.push_str(text);
        self.column += text.len();
        self.line_has_content = true;
    }

    fn push_text(&mut self, text: &str, metrics: TextMetrics, output: &mut String) {
        if !self.line_has_content {
            self.push_indent(output);
        }

        output.push_str(text);
        if let Some(width) = metrics.single_line_width() {
            self.column += width;
            self.line_has_content = true;
        } else {
            self.column = if metrics.ends_with_newline() {
                0
            } else {
                metrics.last_line_width()
            };
            self.line_has_content = !metrics.ends_with_newline();
        }
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

        match self.options.indent_style {
            IndentStyle::Tab => {
                for _ in 0..self.indent_level {
                    output.push('\t');
                }
            }
            IndentStyle::Space => {
                for _ in 0..(self.indent_level * usize::from(self.options.indent_width)) {
                    output.push(' ');
                }
            }
        }

        self.column += self.indent_width();
        self.line_has_content = true;
    }

    fn indent_width(&self) -> usize {
        self.indent_level * usize::from(self.options.indent_width)
    }
}
