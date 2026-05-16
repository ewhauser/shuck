use crate::format_element::FormatElement;

pub trait Buffer {
    fn write_element(&mut self, element: FormatElement);

    fn write_elements<I>(&mut self, elements: I)
    where
        I: IntoIterator<Item = FormatElement>,
    {
        for element in elements {
            self.write_element(element);
        }
    }

    fn elements(&self) -> &[FormatElement];
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VecBuffer {
    elements: Vec<FormatElement>,
}

impl VecBuffer {
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
    pub fn as_slice(&self) -> &[FormatElement] {
        &self.elements
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<FormatElement> {
        self.elements
    }
}

impl Buffer for VecBuffer {
    fn write_element(&mut self, element: FormatElement) {
        self.elements.push(element);
    }

    fn elements(&self) -> &[FormatElement] {
        self.as_slice()
    }
}
