use shuck_format::FormatContext;

use crate::comments::Comments;
use crate::options::ResolvedShellFormatOptions;

#[derive(Debug, Clone)]
pub struct ShellFormatContext<'a> {
    options: ResolvedShellFormatOptions,
    source: &'a str,
    comments: Comments<'a>,
}

impl<'a> ShellFormatContext<'a> {
    #[must_use]
    pub fn new(
        options: ResolvedShellFormatOptions,
        source: &'a str,
        comments: Comments<'a>,
    ) -> Self {
        Self {
            options,
            source,
            comments,
        }
    }

    #[must_use]
    pub fn source(&self) -> &'a str {
        self.source
    }

    #[must_use]
    pub fn options(&self) -> &ResolvedShellFormatOptions {
        &self.options
    }

    pub fn comments_mut(&mut self) -> &mut Comments<'a> {
        &mut self.comments
    }
}

impl FormatContext for ShellFormatContext<'_> {
    type Options = ResolvedShellFormatOptions;

    fn options(&self) -> &Self::Options {
        &self.options
    }
}
