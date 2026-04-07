use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineMode {
    Hard,
    Soft,
    SoftOrSpace,
}

#[derive(Clone, PartialEq, Eq)]
pub enum FormatElement {
    Text(String),
    Space,
    Line(LineMode),
    Indent(Document),
    Group(Document),
    BestFit { flat: Document, expanded: Document },
    Verbatim(String),
}

impl fmt::Debug for FormatElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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

#[must_use]
pub fn text(text: impl Into<String>) -> FormatElement {
    FormatElement::Text(text.into())
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
    FormatElement::Indent(document)
}

#[must_use]
pub fn group(document: Document) -> FormatElement {
    FormatElement::Group(document)
}

#[must_use]
pub fn best_fit(flat: Document, expanded: Document) -> FormatElement {
    FormatElement::BestFit { flat, expanded }
}

#[must_use]
pub fn verbatim(text: impl Into<String>) -> FormatElement {
    FormatElement::Verbatim(text.into())
}
