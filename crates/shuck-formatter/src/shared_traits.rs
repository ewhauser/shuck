use shuck_format::{Format, FormatResult};

use crate::{FormatNodeRule, ShellFormatter, context::ShellFormatContext};

pub struct FormatRefWithRule<'a, T, R> {
    item: &'a T,
    rule: R,
}

impl<'a, T, R> FormatRefWithRule<'a, T, R> {
    #[must_use]
    pub fn new(item: &'a T, rule: R) -> Self {
        Self { item, rule }
    }
}

impl<'source, T, R> Format<ShellFormatContext<'source>> for FormatRefWithRule<'_, T, R>
where
    R: FormatNodeRule<T>,
{
    fn fmt(&self, formatter: &mut ShellFormatter<'source, '_>) -> FormatResult<()> {
        self.rule.fmt(self.item, formatter)
    }
}

#[allow(dead_code)]
pub struct FormatOwnedWithRule<T, R> {
    item: T,
    rule: R,
}

impl<T, R> FormatOwnedWithRule<T, R> {
    #[allow(dead_code)]
    #[must_use]
    pub fn new(item: T, rule: R) -> Self {
        Self { item, rule }
    }
}

impl<'source, T, R> Format<ShellFormatContext<'source>> for FormatOwnedWithRule<T, R>
where
    R: FormatNodeRule<T>,
{
    fn fmt(&self, formatter: &mut ShellFormatter<'source, '_>) -> FormatResult<()> {
        self.rule.fmt(&self.item, formatter)
    }
}

pub trait AsFormat<'a> {
    type Format: Format<ShellFormatContext<'a>>;

    fn format(&'a self) -> Self::Format;
}

#[allow(dead_code)]
pub trait IntoFormat<'a> {
    type Format: Format<ShellFormatContext<'a>>;

    fn into_format(self) -> Self::Format;
}
