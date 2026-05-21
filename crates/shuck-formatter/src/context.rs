use crate::comments::SourceMap;
use crate::facts::FormatterFacts;
use crate::options::ResolvedShellFormatOptions;

#[derive(Clone, Copy)]
pub(crate) struct RenderContext<'source, 'a> {
    pub(crate) source: &'source str,
    pub(crate) options: &'a ResolvedShellFormatOptions,
    pub(crate) facts: &'a FormatterFacts<'source>,
}

impl<'source, 'a> RenderContext<'source, 'a> {
    pub(crate) fn new(
        source: &'source str,
        options: &'a ResolvedShellFormatOptions,
        facts: &'a FormatterFacts<'source>,
    ) -> Self {
        Self {
            source,
            options,
            facts,
        }
    }

    pub(crate) fn source_map(self) -> &'a SourceMap<'source> {
        self.facts.source_map()
    }
}
