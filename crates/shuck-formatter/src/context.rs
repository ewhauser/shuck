use crate::Result;
use crate::comments::SourceMap;
use crate::facts::FormatterFacts;
use crate::options::ResolvedShellFormatOptions;
use shuck_ast::{File, StmtSeq};
use shuck_parser::parser::Parser;

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

    pub(crate) fn format_stmt_sequence_to_buf(
        self,
        statements: &StmtSeq,
        upper_bound: Option<usize>,
        output: &mut String,
    ) -> Result<()> {
        crate::streaming::format_stmt_sequence_streaming_to_buf(
            self,
            statements,
            upper_bound,
            output,
        )
    }
}

pub(crate) struct FragmentFormatter<'source, 'a> {
    source: &'source str,
    options: &'a ResolvedShellFormatOptions,
    file: File,
    facts: FormatterFacts<'source>,
}

impl<'source, 'a> FragmentFormatter<'source, 'a> {
    pub(crate) fn parse(
        source: &'source str,
        options: &'a ResolvedShellFormatOptions,
    ) -> Option<Self> {
        let parsed = Parser::with_dialect(source, options.dialect()).parse();
        if parsed.is_err() {
            return None;
        }

        let file = parsed.file;
        let facts = FormatterFacts::build(source, &file, options);
        Some(Self {
            source,
            options,
            file,
            facts,
        })
    }

    pub(crate) fn body(&self) -> &StmtSeq {
        &self.file.body
    }

    pub(crate) fn body_len(&self) -> usize {
        self.file.body.len()
    }

    pub(crate) fn facts(&self) -> &FormatterFacts<'source> {
        &self.facts
    }

    pub(crate) fn render_context(&self) -> RenderContext<'source, '_> {
        RenderContext::new(self.source, self.options, &self.facts)
    }

    pub(crate) fn format_body_to_buf(
        &self,
        upper_bound: Option<usize>,
        output: &mut String,
    ) -> Result<()> {
        self.render_context()
            .format_stmt_sequence_to_buf(self.body(), upper_bound, output)
    }

    pub(crate) fn format_body(&self, upper_bound: Option<usize>) -> Result<String> {
        let mut output = String::new();
        self.format_body_to_buf(upper_bound, &mut output)?;
        Ok(output)
    }
}
