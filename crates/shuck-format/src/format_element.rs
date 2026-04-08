use std::fmt;
use std::sync::Arc;

use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineMode {
    Hard,
    Soft,
    SoftOrSpace,
}

#[derive(Clone, PartialEq, Eq)]
pub enum FormatElement {
    Token(&'static str),
    Text(TextElement),
    Space,
    Line(LineMode),
    Indent(InternedDocument),
    Group(InternedDocument),
    BestFit {
        flat: InternedDocument,
        expanded: InternedDocument,
    },
    Verbatim(VerbatimText),
}

impl fmt::Debug for FormatElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Token(text) => f.debug_tuple("Token").field(text).finish(),
            Self::Text(text) => f.debug_tuple("Text").field(text).finish(),
            Self::Space => f.write_str("Space"),
            Self::Line(mode) => f.debug_tuple("Line").field(mode).finish(),
            Self::Indent(document) => f.debug_tuple("Indent").field(document).finish(),
            Self::Group(document) => f.debug_tuple("Group").field(document).finish(),
            Self::BestFit { flat, expanded } => f
                .debug_struct("BestFit")
                .field("flat", flat)
                .field("expanded", expanded)
                .finish(),
            Self::Verbatim(text) => f.debug_tuple("Verbatim").field(text).finish(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    elements: Vec<FormatElement>,
}

impl Document {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            elements: Vec::with_capacity(capacity),
        }
    }

    #[must_use]
    pub fn from_element(element: FormatElement) -> Self {
        Self {
            elements: vec![element],
        }
    }

    #[must_use]
    pub fn from_elements(elements: Vec<FormatElement>) -> Self {
        Self { elements }
    }

    pub fn push(&mut self, element: FormatElement) {
        self.elements.push(element);
    }

    pub fn extend(&mut self, document: Document) {
        self.elements.extend(document.elements);
    }

    #[must_use]
    pub fn as_slice(&self) -> &[FormatElement] {
        &self.elements
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<FormatElement> {
        self.elements
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct InternedDocument {
    elements: Arc<[FormatElement]>,
}

impl InternedDocument {
    #[must_use]
    pub fn as_slice(&self) -> &[FormatElement] {
        &self.elements
    }
}

impl From<Document> for InternedDocument {
    fn from(document: Document) -> Self {
        Self {
            elements: Arc::from(document.elements),
        }
    }
}

impl fmt::Debug for InternedDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.elements.iter()).finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TextElement {
    text: Box<str>,
    metrics: TextMetrics,
}

impl TextElement {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into().into_boxed_str();
        let metrics = TextMetrics::from_text(&text, 4);
        Self { text, metrics }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn metrics(&self) -> TextMetrics {
        self.metrics
    }
}

impl fmt::Debug for TextElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextElement")
            .field("text", &self.text)
            .field("metrics", &self.metrics)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct VerbatimText {
    text: Box<str>,
    metrics: TextMetrics,
}

impl VerbatimText {
    #[must_use]
    pub fn new(text: impl Into<String>, indent_width: u8) -> Self {
        let text = text.into().into_boxed_str();
        let metrics = TextMetrics::from_text(&text, indent_width);
        Self { text, metrics }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn metrics(&self) -> TextMetrics {
        self.metrics
    }
}

impl fmt::Debug for VerbatimText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerbatimText")
            .field("text", &self.text)
            .field("metrics", &self.metrics)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextMetrics {
    first_line_width: usize,
    last_line_width: usize,
    single_line_width: Option<usize>,
    has_newline: bool,
    ends_with_newline: bool,
}

impl TextMetrics {
    #[must_use]
    pub fn from_text(text: &str, indent_width: u8) -> Self {
        let mut current_width = 0usize;
        let mut first_line_width = 0usize;
        let mut saw_newline = false;
        let mut ends_with_newline = false;

        for ch in text.chars() {
            match ch {
                '\n' => {
                    if !saw_newline {
                        first_line_width = current_width;
                    }
                    current_width = 0;
                    saw_newline = true;
                    ends_with_newline = true;
                }
                '\t' => {
                    current_width += usize::from(indent_width);
                    ends_with_newline = false;
                }
                _ => {
                    current_width += unicode_width(ch);
                    ends_with_newline = false;
                }
            }
        }

        if !saw_newline {
            first_line_width = current_width;
        }

        Self {
            first_line_width,
            last_line_width: current_width,
            single_line_width: (!saw_newline).then_some(current_width),
            has_newline: saw_newline,
            ends_with_newline,
        }
    }

    #[must_use]
    pub fn first_line_width(self) -> usize {
        self.first_line_width
    }

    #[must_use]
    pub fn last_line_width(self) -> usize {
        self.last_line_width
    }

    #[must_use]
    pub fn single_line_width(self) -> Option<usize> {
        self.single_line_width
    }

    #[must_use]
    pub fn has_newline(self) -> bool {
        self.has_newline
    }

    #[must_use]
    pub fn ends_with_newline(self) -> bool {
        self.ends_with_newline
    }
}

fn unicode_width(ch: char) -> usize {
    ch.width().unwrap_or(0)
}

#[must_use]
pub const fn token(text: &'static str) -> FormatElement {
    FormatElement::Token(text)
}

#[must_use]
pub fn text(text: impl Into<String>) -> FormatElement {
    FormatElement::Text(TextElement::new(text))
}

#[must_use]
pub const fn space() -> FormatElement {
    FormatElement::Space
}

#[must_use]
pub const fn hard_line_break() -> FormatElement {
    FormatElement::Line(LineMode::Hard)
}

#[must_use]
pub const fn soft_line_break() -> FormatElement {
    FormatElement::Line(LineMode::Soft)
}

#[must_use]
pub const fn soft_line_break_or_space() -> FormatElement {
    FormatElement::Line(LineMode::SoftOrSpace)
}

#[must_use]
pub fn indent(document: Document) -> FormatElement {
    FormatElement::Indent(InternedDocument::from(document))
}

#[must_use]
pub fn group(document: Document) -> FormatElement {
    FormatElement::Group(InternedDocument::from(document))
}

#[must_use]
pub fn best_fit(flat: Document, expanded: Document) -> FormatElement {
    FormatElement::BestFit {
        flat: InternedDocument::from(flat),
        expanded: InternedDocument::from(expanded),
    }
}

#[must_use]
pub fn verbatim(text: impl Into<String>) -> FormatElement {
    FormatElement::Verbatim(VerbatimText::new(text, 4))
}
