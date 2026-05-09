use super::*;
use shuck_ast::{
    AllElementsArrayExpansionKind, AllElementsArrayExpansionOrigin,
    AllElementsArrayExpansionSyntax, EscapedParameterTemplateSyntax, WordSurfaceSyntax,
    ZshShortPositionalAtKind, ZshShortPositionalAtSyntax, ZshWordSurfaceSyntax,
};

impl<'a> Parser<'a> {
    fn word_surface_interest(&self, word_span: Span) -> WordSurfaceInterest {
        let brace_syntax_enabled = self.brace_syntax_enabled_at(word_span.start.offset);
        let Some(text) = self.input.get(word_span.start.offset..word_span.end.offset) else {
            return WordSurfaceInterest {
                brace_syntax: brace_syntax_enabled,
                ..WordSurfaceInterest::default()
            };
        };

        let bytes = text.as_bytes();
        WordSurfaceInterest {
            brace_syntax: brace_syntax_enabled && memchr(b'{', bytes).is_some(),
            escaped_parameter_templates: has_escaped_parameter_template(text),
            all_elements_array_expansions: memchr2(b'$', b'@', bytes).is_some(),
            zsh_short_positional_at: self.dialect == ShellDialect::Zsh
                && has_zsh_short_positional_at(text),
        }
    }

    pub(super) fn word_surface_syntax_from_parts(
        &self,
        parts: &[WordPartNode],
        word_span: Span,
    ) -> WordSurfaceSyntax {
        if let [part] = parts
            && let Some(surface_syntax) = self.fast_surface_syntax_for_single_part(part)
        {
            return surface_syntax;
        }

        let interest = self.word_surface_interest(word_span);
        if interest.is_empty() {
            return WordSurfaceSyntax::default();
        }

        let braces = if interest.brace_syntax {
            self.brace_syntax_from_parts(parts, word_span.start.offset)
        } else {
            Vec::new()
        };
        let escaped_parameter_templates = if interest.escaped_parameter_templates {
            self.collect_escaped_parameter_templates_in_span(word_span)
        } else {
            Vec::new()
        };
        let all_elements_array_expansions = if interest.all_elements_array_expansions {
            self.collect_all_elements_array_expansions_from_parts(
                parts,
                escaped_parameter_templates.as_slice(),
            )
        } else {
            Vec::new()
        };

        WordSurfaceSyntax {
            braces,
            escaped_parameter_templates,
            all_elements_array_expansions,
        }
    }

    pub(super) fn zsh_word_surface_syntax_from_parts(
        &self,
        parts: &[WordPartNode],
        word_span: Span,
        escaped_parameter_templates: &[EscapedParameterTemplateSyntax],
    ) -> Option<ZshWordSurfaceSyntax> {
        if self.dialect != ShellDialect::Zsh || parts.len() < 2 {
            return None;
        }

        let Some(text) = self.input.get(word_span.start.offset..word_span.end.offset) else {
            return None;
        };
        if !has_zsh_short_positional_at(text) {
            return None;
        }

        let mut short_positional_at = Vec::new();
        self.collect_zsh_short_positional_at_from_parts(
            parts,
            BraceQuoteContext::Unquoted,
            escaped_parameter_templates,
            &mut short_positional_at,
        );
        if short_positional_at.len() > 1 {
            short_positional_at.sort_by_key(|entry| {
                (
                    entry.span.start.offset,
                    entry.span.end.offset,
                    entry.base_span.start.offset,
                )
            });
            short_positional_at.dedup_by_key(|entry| {
                (
                    entry.span.start.offset,
                    entry.span.end.offset,
                    entry.base_span.start.offset,
                    entry.suffix_span.start.offset,
                    entry.kind,
                    entry.quote_context,
                )
            });
        }

        (!short_positional_at.is_empty()).then_some(ZshWordSurfaceSyntax {
            short_positional_at,
        })
    }

    fn fast_surface_syntax_for_single_part(
        &self,
        part: &WordPartNode,
    ) -> Option<WordSurfaceSyntax> {
        match &part.kind {
            WordPart::Literal(text) => {
                let syntax_text = text.syntax_str(self.input, part.span);
                let mut surface_syntax = WordSurfaceSyntax::default();

                if self.brace_syntax_enabled_at(part.span.start.offset)
                    && memchr(b'{', syntax_text.as_bytes()).is_some()
                {
                    let mut braces = Vec::new();
                    Self::scan_brace_syntax_text(
                        syntax_text,
                        part.span.start,
                        BraceQuoteContext::Unquoted,
                        self.brace_ccl_enabled_at(part.span.start.offset),
                        &mut braces,
                    );
                    surface_syntax.braces = braces;
                }

                if has_escaped_parameter_template(syntax_text) {
                    surface_syntax.escaped_parameter_templates =
                        self.collect_escaped_parameter_templates_in_span(part.span);
                }

                Some(surface_syntax)
            }
            WordPart::SingleQuoted { .. } => {
                let Some(raw) = self.input.get(part.span.start.offset..part.span.end.offset) else {
                    return Some(WordSurfaceSyntax::default());
                };
                let mut surface_syntax = WordSurfaceSyntax::default();

                if self.brace_syntax_enabled_at(part.span.start.offset)
                    && memchr(b'{', raw.as_bytes()).is_some()
                {
                    let mut braces = Vec::new();
                    Self::scan_brace_syntax_text(
                        raw,
                        part.span.start,
                        BraceQuoteContext::SingleQuoted,
                        self.brace_ccl_enabled_at(part.span.start.offset),
                        &mut braces,
                    );
                    surface_syntax.braces = braces;
                }

                if has_escaped_parameter_template(raw) {
                    surface_syntax.escaped_parameter_templates =
                        self.collect_escaped_parameter_templates_in_span(part.span);
                }

                Some(surface_syntax)
            }
            WordPart::Variable(name) => {
                let kind = match name.as_str() {
                    "@" => Some(AllElementsArrayExpansionKind::PositionalAt),
                    "*" => Some(AllElementsArrayExpansionKind::PositionalStar),
                    _ => None,
                };
                Some(WordSurfaceSyntax {
                    braces: Vec::new(),
                    escaped_parameter_templates: Vec::new(),
                    all_elements_array_expansions: kind
                        .into_iter()
                        .map(|kind| AllElementsArrayExpansionSyntax {
                            span: part.span,
                            kind,
                            origin: AllElementsArrayExpansionOrigin::DirectPart,
                            direct: true,
                            quote_context: BraceQuoteContext::Unquoted,
                        })
                        .collect(),
                })
            }
            WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::ProcessSubstitution { .. } => Some(WordSurfaceSyntax::default()),
            WordPart::DoubleQuoted { .. }
            | WordPart::ZshQualifiedGlob(_)
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::Transformation { .. } => None,
        }
    }

    fn collect_escaped_parameter_templates_in_span(
        &self,
        word_span: Span,
    ) -> Vec<EscapedParameterTemplateSyntax> {
        let Some(text) = self.input.get(word_span.start.offset..word_span.end.offset) else {
            return Vec::new();
        };

        #[derive(Clone, Copy, PartialEq, Eq)]
        enum QuoteState {
            Single,
            Double,
        }

        let mut templates = Vec::new();
        let mut index = 0usize;
        let mut quote_state = None;

        while index < text.len() {
            if text[index..].starts_with("\\${") {
                let dollar_offset = index + '\\'.len_utf8();
                if offset_is_backslash_escaped(word_span.start.offset + dollar_offset, self.input)
                    && let Some(end_offset) = escaped_parameter_template_end(text, dollar_offset)
                {
                    let full_span = Span::from_positions(
                        word_span.start.advanced_by(&text[..index]),
                        word_span.start.advanced_by(&text[..end_offset]),
                    );
                    let body_start = dollar_offset + "${".len();
                    let body_end = end_offset.saturating_sub('}'.len_utf8());
                    let body_span = Span::from_positions(
                        word_span.start.advanced_by(&text[..body_start]),
                        word_span.start.advanced_by(&text[..body_end]),
                    );
                    templates.push(EscapedParameterTemplateSyntax {
                        span: full_span,
                        body_span,
                        quote_context: match quote_state {
                            None => BraceQuoteContext::Unquoted,
                            Some(QuoteState::Single) => BraceQuoteContext::SingleQuoted,
                            Some(QuoteState::Double) => BraceQuoteContext::DoubleQuoted,
                        },
                        contains_nested_parameter: text[body_start..body_end].contains("${"),
                    });
                    index = end_offset;
                    continue;
                }
            }

            let Some(ch) = text[index..].chars().next() else {
                break;
            };
            match quote_state {
                None => match ch {
                    '\'' => quote_state = Some(QuoteState::Single),
                    '"' => quote_state = Some(QuoteState::Double),
                    _ => {}
                },
                Some(QuoteState::Single) => {
                    if ch == '\'' {
                        quote_state = None;
                    }
                }
                Some(QuoteState::Double) => {
                    if ch == '\\' {
                        index += ch.len_utf8();
                        if let Some(next) = text[index..].chars().next() {
                            index += next.len_utf8();
                        }
                        continue;
                    }
                    if ch == '"' {
                        quote_state = None;
                    }
                }
            }
            index += ch.len_utf8();
        }

        if templates.len() > 1 {
            templates.sort_by_key(|template| {
                (
                    template.span.start.offset,
                    template.span.end.offset,
                    template.body_span.start.offset,
                )
            });
            templates.dedup_by_key(|template| {
                (
                    template.span.start.offset,
                    template.span.end.offset,
                    template.body_span.start.offset,
                    template.body_span.end.offset,
                    template.quote_context,
                )
            });
        }
        templates
    }

    fn collect_all_elements_array_expansions_from_parts(
        &self,
        parts: &[WordPartNode],
        escaped_parameter_templates: &[EscapedParameterTemplateSyntax],
    ) -> Vec<AllElementsArrayExpansionSyntax> {
        let mut expansions = Vec::new();
        self.collect_all_elements_array_expansions_recursive(
            parts,
            BraceQuoteContext::Unquoted,
            escaped_parameter_templates,
            &mut expansions,
        );
        if expansions.len() > 1 {
            expansions.sort_by_key(|entry| {
                (
                    entry.span.start.offset,
                    entry.span.end.offset,
                    entry.kind as u8,
                    entry.origin as u8,
                    entry.direct,
                )
            });
            expansions.dedup_by_key(|entry| {
                (
                    entry.span.start.offset,
                    entry.span.end.offset,
                    entry.kind,
                    entry.origin,
                    entry.direct,
                    entry.quote_context,
                )
            });
        }
        expansions
    }

    fn collect_all_elements_array_expansions_recursive(
        &self,
        parts: &[WordPartNode],
        quote_context: BraceQuoteContext,
        escaped_parameter_templates: &[EscapedParameterTemplateSyntax],
        out: &mut Vec<AllElementsArrayExpansionSyntax>,
    ) {
        for (index, part) in parts.iter().enumerate() {
            if span_start_inside_escaped_parameter_template(part.span, escaped_parameter_templates)
            {
                continue;
            }

            match &part.kind {
                WordPart::SingleQuoted { .. } => {}
                WordPart::DoubleQuoted { parts, .. } => {
                    self.collect_all_elements_array_expansions_recursive(
                        parts,
                        BraceQuoteContext::DoubleQuoted,
                        escaped_parameter_templates,
                        out,
                    );
                }
                WordPart::Parameter(parameter) => {
                    let direct_kind =
                        direct_all_elements_kind_for_parameter(parameter, self.dialect);
                    if let Some(kind) = all_elements_kind_for_parameter(parameter, self.dialect) {
                        out.push(AllElementsArrayExpansionSyntax {
                            span: part.span,
                            kind,
                            origin: AllElementsArrayExpansionOrigin::DirectPart,
                            direct: direct_kind.is_some(),
                            quote_context,
                        });
                        continue;
                    }

                    let candidate = part.span.slice(self.input);
                    if let Some(kind) =
                        candidate_all_elements_array_expansion_kind(candidate, self.dialect)
                    {
                        out.push(AllElementsArrayExpansionSyntax {
                            span: part.span,
                            kind,
                            origin: AllElementsArrayExpansionOrigin::DirectPart,
                            direct: candidate_direct_all_elements_array_expansion_kind(
                                candidate,
                                self.dialect,
                            )
                            .is_some(),
                            quote_context,
                        });
                        continue;
                    }

                    if let Some((span, kind)) = nested_all_elements_array_expansion_in_parameter(
                        parameter,
                        part.span,
                        self.input,
                        self.dialect,
                    ) {
                        out.push(AllElementsArrayExpansionSyntax {
                            span,
                            kind,
                            origin: AllElementsArrayExpansionOrigin::NestedParameterBody,
                            direct: false,
                            quote_context,
                        });
                    }
                }
                _ => {
                    if let Some((kind, direct)) = all_elements_surface_for_part(
                        parts,
                        index,
                        &part.kind,
                        self.input,
                        self.dialect,
                    ) {
                        out.push(AllElementsArrayExpansionSyntax {
                            span: part.span,
                            kind,
                            origin: AllElementsArrayExpansionOrigin::DirectPart,
                            direct,
                            quote_context,
                        });
                    } else if matches!(
                        &part.kind,
                        WordPart::ParameterExpansion { .. }
                            | WordPart::IndirectExpansion { .. }
                            | WordPart::Transformation { .. }
                            | WordPart::PrefixMatch { .. }
                    ) && let Some((span, kind)) =
                        first_all_elements_array_expansion_in_text(
                            part.span,
                            self.input,
                            self.dialect,
                        )
                    {
                        let direct = span == part.span
                            && candidate_direct_all_elements_array_expansion_kind(
                                part.span.slice(self.input),
                                self.dialect,
                            )
                            .is_some();
                        out.push(AllElementsArrayExpansionSyntax {
                            span,
                            kind,
                            origin: if span == part.span {
                                AllElementsArrayExpansionOrigin::DirectPart
                            } else {
                                AllElementsArrayExpansionOrigin::NestedParameterBody
                            },
                            direct,
                            quote_context,
                        });
                    }
                }
            }
        }
    }

    fn collect_zsh_short_positional_at_from_parts(
        &self,
        parts: &[WordPartNode],
        quote_context: BraceQuoteContext,
        escaped_parameter_templates: &[EscapedParameterTemplateSyntax],
        out: &mut Vec<ZshShortPositionalAtSyntax>,
    ) {
        for (index, part) in parts.iter().enumerate() {
            if span_start_inside_escaped_parameter_template(part.span, escaped_parameter_templates)
            {
                continue;
            }

            match &part.kind {
                WordPart::SingleQuoted { .. } => {}
                WordPart::DoubleQuoted { parts, .. } => {
                    self.collect_zsh_short_positional_at_from_parts(
                        parts,
                        BraceQuoteContext::DoubleQuoted,
                        escaped_parameter_templates,
                        out,
                    );
                }
                WordPart::Variable(name) if name.as_str() == "@" => {
                    let Some(next) = parts.get(index + 1) else {
                        continue;
                    };
                    let WordPart::Literal(text) = &next.kind else {
                        continue;
                    };
                    let literal = text.as_str(self.input, next.span);
                    let Some((suffix_len, kind)) = zsh_short_positional_at_suffix(literal) else {
                        continue;
                    };
                    let suffix_span = Span::from_positions(
                        next.span.start,
                        next.span.start.advanced_by(&literal[..suffix_len]),
                    );
                    out.push(ZshShortPositionalAtSyntax {
                        span: part.span.merge(suffix_span),
                        base_span: part.span,
                        suffix_span,
                        kind,
                        quote_context,
                    });
                }
                _ => {}
            }
        }
    }

    fn brace_syntax_from_parts(&self, parts: &[WordPartNode], offset: usize) -> Vec<BraceSyntax> {
        if !self.brace_syntax_enabled_at(offset) {
            return Vec::new();
        }
        let brace_ccl_enabled = self.brace_ccl_enabled_at(offset);
        let mut brace_syntax = Vec::new();
        self.collect_brace_syntax_from_parts(
            parts,
            BraceQuoteContext::Unquoted,
            brace_ccl_enabled,
            &mut brace_syntax,
        );
        if brace_syntax.len() > 1 {
            brace_syntax.sort_by_key(|brace| (brace.span.start.offset, brace.span.end.offset));
            brace_syntax.dedup_by_key(|brace| {
                (
                    brace.span.start.offset,
                    brace.span.end.offset,
                    brace.quote_context,
                    brace.kind,
                )
            });
        }
        brace_syntax
    }

    fn collect_brace_syntax_from_parts(
        &self,
        parts: &[WordPartNode],
        quote_context: BraceQuoteContext,
        brace_ccl_enabled: bool,
        out: &mut Vec<BraceSyntax>,
    ) {
        for part in parts {
            match &part.kind {
                WordPart::Literal(text) => Self::scan_brace_syntax_text(
                    text.syntax_str(self.input, part.span),
                    part.span.start,
                    quote_context,
                    brace_ccl_enabled,
                    out,
                ),
                WordPart::SingleQuoted { .. } => Self::scan_brace_syntax_text(
                    part.span.slice(self.input),
                    part.span.start,
                    BraceQuoteContext::SingleQuoted,
                    brace_ccl_enabled,
                    out,
                ),
                WordPart::DoubleQuoted { parts, .. } => self.collect_brace_syntax_from_parts(
                    parts,
                    BraceQuoteContext::DoubleQuoted,
                    brace_ccl_enabled,
                    out,
                ),
                WordPart::ZshQualifiedGlob(glob) => self
                    .collect_brace_syntax_from_zsh_qualified_glob(
                        glob,
                        quote_context,
                        brace_ccl_enabled,
                        out,
                    ),
                WordPart::Variable(_)
                | WordPart::CommandSubstitution { .. }
                | WordPart::ArithmeticExpansion { .. }
                | WordPart::Parameter(_)
                | WordPart::ParameterExpansion { .. }
                | WordPart::Length(_)
                | WordPart::ArrayAccess(_)
                | WordPart::ArrayLength(_)
                | WordPart::ArrayIndices(_)
                | WordPart::Substring { .. }
                | WordPart::ArraySlice { .. }
                | WordPart::IndirectExpansion { .. }
                | WordPart::PrefixMatch { .. }
                | WordPart::ProcessSubstitution { .. }
                | WordPart::Transformation { .. } => {}
            }
        }

        if self.needs_cross_part_brace_scan(parts) {
            let mut chars = Vec::new();
            self.collect_brace_scan_chars_from_parts(parts, &mut chars);
            Self::scan_brace_syntax_chars(&chars, quote_context, brace_ccl_enabled, out);
        }
    }

    fn needs_cross_part_brace_scan(&self, parts: &[WordPartNode]) -> bool {
        if parts.len() < 2 {
            return false;
        }

        let mut cursor = parts[0].span.start.offset;
        for part in parts {
            if part.span.start.offset > cursor
                && self
                    .input
                    .get(cursor..part.span.start.offset)
                    .is_some_and(|raw| raw.contains(['{', '}']))
            {
                return true;
            }

            let has_brace_text = match &part.kind {
                WordPart::Literal(text) => {
                    text.syntax_str(self.input, part.span).contains(['{', '}'])
                }
                WordPart::SingleQuoted { .. }
                | WordPart::DoubleQuoted { .. }
                | WordPart::ZshQualifiedGlob(_) => self
                    .input
                    .get(part.span.start.offset..part.span.end.offset)
                    .is_some_and(|raw| raw.contains(['{', '}'])),
                WordPart::Variable(_)
                | WordPart::CommandSubstitution { .. }
                | WordPart::ArithmeticExpansion { .. }
                | WordPart::Parameter(_)
                | WordPart::ParameterExpansion { .. }
                | WordPart::Length(_)
                | WordPart::ArrayAccess(_)
                | WordPart::ArrayLength(_)
                | WordPart::ArrayIndices(_)
                | WordPart::Substring { .. }
                | WordPart::ArraySlice { .. }
                | WordPart::IndirectExpansion { .. }
                | WordPart::PrefixMatch { .. }
                | WordPart::ProcessSubstitution { .. }
                | WordPart::Transformation { .. } => false,
            };
            if has_brace_text {
                return true;
            }

            cursor = part.span.end.offset;
        }

        false
    }

    fn collect_brace_scan_chars_from_parts(
        &self,
        parts: &[WordPartNode],
        out: &mut Vec<(char, Position)>,
    ) {
        for part in parts {
            match &part.kind {
                WordPart::Literal(text) => {
                    Self::push_brace_scan_text(
                        text.syntax_str(self.input, part.span),
                        part.span.start,
                        out,
                    );
                }
                WordPart::SingleQuoted { .. } => {
                    if let Some(raw) = self.input.get(part.span.start.offset..part.span.end.offset)
                    {
                        Self::push_brace_scan_text(raw, part.span.start, out);
                    }
                }
                WordPart::DoubleQuoted { parts, .. } => {
                    self.collect_brace_scan_chars_from_double_quoted_part(part.span, parts, out);
                }
                WordPart::ZshQualifiedGlob(_) => {}
                WordPart::Variable(_)
                | WordPart::CommandSubstitution { .. }
                | WordPart::ArithmeticExpansion { .. }
                | WordPart::Parameter(_)
                | WordPart::ParameterExpansion { .. }
                | WordPart::Length(_)
                | WordPart::ArrayAccess(_)
                | WordPart::ArrayLength(_)
                | WordPart::ArrayIndices(_)
                | WordPart::Substring { .. }
                | WordPart::ArraySlice { .. }
                | WordPart::IndirectExpansion { .. }
                | WordPart::PrefixMatch { .. }
                | WordPart::ProcessSubstitution { .. }
                | WordPart::Transformation { .. } => {
                    Self::push_brace_scan_boundary(part.span.start, out);
                }
            }
        }
    }

    fn collect_brace_scan_chars_from_double_quoted_part(
        &self,
        span: Span,
        parts: &[WordPartNode],
        out: &mut Vec<(char, Position)>,
    ) {
        let mut cursor_offset = span.start.offset;
        let mut cursor_position = span.start;

        for part in parts {
            if part.span.start.offset > cursor_offset
                && let Some(raw) = self.input.get(cursor_offset..part.span.start.offset)
            {
                Self::push_brace_scan_text(raw, cursor_position, out);
            }

            match &part.kind {
                WordPart::Literal(text) => {
                    Self::push_brace_scan_text(
                        text.syntax_str(self.input, part.span),
                        part.span.start,
                        out,
                    );
                }
                WordPart::SingleQuoted { .. } => {
                    if let Some(raw) = self.input.get(part.span.start.offset..part.span.end.offset)
                    {
                        Self::push_brace_scan_text(raw, part.span.start, out);
                    }
                }
                WordPart::DoubleQuoted { parts, .. } => {
                    self.collect_brace_scan_chars_from_double_quoted_part(part.span, parts, out);
                }
                WordPart::ZshQualifiedGlob(_) => {}
                WordPart::Variable(_)
                | WordPart::CommandSubstitution { .. }
                | WordPart::ArithmeticExpansion { .. }
                | WordPart::Parameter(_)
                | WordPart::ParameterExpansion { .. }
                | WordPart::Length(_)
                | WordPart::ArrayAccess(_)
                | WordPart::ArrayLength(_)
                | WordPart::ArrayIndices(_)
                | WordPart::Substring { .. }
                | WordPart::ArraySlice { .. }
                | WordPart::IndirectExpansion { .. }
                | WordPart::PrefixMatch { .. }
                | WordPart::ProcessSubstitution { .. }
                | WordPart::Transformation { .. } => {
                    Self::push_brace_scan_boundary(part.span.start, out);
                }
            }

            cursor_offset = part.span.end.offset;
            cursor_position = part.span.end;
        }

        if cursor_offset < span.end.offset
            && let Some(raw) = self.input.get(cursor_offset..span.end.offset)
        {
            Self::push_brace_scan_text(raw, cursor_position, out);
        }
    }

    fn push_brace_scan_text(text: &str, start: Position, out: &mut Vec<(char, Position)>) {
        let mut position = start;
        for ch in text.chars() {
            out.push((ch, position));
            position.advance(ch);
        }
    }

    fn push_brace_scan_boundary(position: Position, out: &mut Vec<(char, Position)>) {
        out.push(('\0', position));
    }

    fn scan_brace_syntax_chars(
        chars: &[(char, Position)],
        initial_quote_context: BraceQuoteContext,
        brace_ccl_enabled: bool,
        out: &mut Vec<BraceSyntax>,
    ) {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum QuoteState {
            Single,
            AnsiSingle,
            Double,
        }

        #[derive(Clone, Copy)]
        struct Candidate {
            start: Position,
            quote_context: BraceQuoteContext,
            has_comma: bool,
            has_dot_dot: bool,
            saw_unquoted_whitespace: bool,
            has_brace_ccl_content: bool,
            saw_quote_boundary: bool,
            prev_char: Option<char>,
        }

        fn quote_state(context: BraceQuoteContext) -> Option<QuoteState> {
            match context {
                BraceQuoteContext::Unquoted => None,
                BraceQuoteContext::SingleQuoted => Some(QuoteState::Single),
                BraceQuoteContext::DoubleQuoted => Some(QuoteState::Double),
            }
        }

        fn quote_context(state: Option<QuoteState>) -> BraceQuoteContext {
            match state {
                None => BraceQuoteContext::Unquoted,
                Some(QuoteState::Single | QuoteState::AnsiSingle) => {
                    BraceQuoteContext::SingleQuoted
                }
                Some(QuoteState::Double) => BraceQuoteContext::DoubleQuoted,
            }
        }

        fn quote_context_is_active(context: BraceQuoteContext, state: Option<QuoteState>) -> bool {
            match context {
                BraceQuoteContext::Unquoted => state.is_none(),
                BraceQuoteContext::SingleQuoted => {
                    matches!(state, Some(QuoteState::Single | QuoteState::AnsiSingle))
                }
                BraceQuoteContext::DoubleQuoted => matches!(state, Some(QuoteState::Double)),
            }
        }

        fn last_active_candidate_index(
            stack: &[Candidate],
            state: Option<QuoteState>,
        ) -> Option<usize> {
            stack
                .iter()
                .rposition(|candidate| quote_context_is_active(candidate.quote_context, state))
        }

        fn mark_brace_ccl_content(stack: &mut [Candidate], state: Option<QuoteState>) {
            if let Some(candidate_index) = last_active_candidate_index(stack, state) {
                stack[candidate_index].has_brace_ccl_content = true;
            }
        }

        fn template_placeholder_end(
            chars: &[(char, Position)],
            start: usize,
            quote_context: BraceQuoteContext,
        ) -> Option<usize> {
            if chars.get(start)?.0 != '{' || chars.get(start + 1)?.0 != '{' {
                return None;
            }

            let mut index = start + 2;
            let mut depth = 1usize;

            while index < chars.len() {
                if matches!(quote_context, BraceQuoteContext::Unquoted) && chars[index].0 == '\\' {
                    index += 1;
                    if index < chars.len() {
                        index += 1;
                    }
                    continue;
                }

                if chars.get(index).is_some_and(|(ch, _)| *ch == '{')
                    && chars.get(index + 1).is_some_and(|(ch, _)| *ch == '{')
                {
                    depth += 1;
                    index += 2;
                    continue;
                }

                if chars.get(index).is_some_and(|(ch, _)| *ch == '}')
                    && chars.get(index + 1).is_some_and(|(ch, _)| *ch == '}')
                {
                    depth -= 1;
                    index += 2;
                    if depth == 0 {
                        return Some(index);
                    }
                    continue;
                }

                index += 1;
            }

            None
        }

        fn brace_span(start: Position, end: (char, Position)) -> Span {
            let mut end_position = end.1;
            end_position.advance(end.0);
            Span::from_positions(start, end_position)
        }

        let mut index = 0usize;
        let mut quote_state = quote_state(initial_quote_context);
        let mut stack = Vec::<Candidate>::new();

        while index < chars.len() {
            let ch = chars[index].0;

            if matches!(
                quote_state,
                None | Some(QuoteState::AnsiSingle) | Some(QuoteState::Double)
            ) && ch == '\\'
            {
                if let Some(candidate_index) = last_active_candidate_index(&stack, quote_state) {
                    if index + 1 < chars.len() {
                        stack[candidate_index].has_brace_ccl_content = true;
                    }
                    stack[candidate_index].prev_char = None;
                }
                index += 1;
                if index < chars.len() {
                    index += 1;
                }
                continue;
            }

            let current_quote_context = quote_context(quote_state);
            if ch == '{'
                && let Some(end_index) =
                    template_placeholder_end(chars, index, current_quote_context)
            {
                out.push(BraceSyntax {
                    kind: BraceSyntaxKind::TemplatePlaceholder,
                    span: brace_span(chars[index].1, chars[end_index - 1]),
                    quote_context: current_quote_context,
                });
                if let Some(candidate_index) = last_active_candidate_index(&stack, quote_state) {
                    stack[candidate_index].prev_char = None;
                }
                index = end_index;
                continue;
            }

            match quote_state {
                None => match ch {
                    '\'' => {
                        quote_state = if index > 0 && chars[index - 1].0 == '$' {
                            Some(QuoteState::AnsiSingle)
                        } else {
                            Some(QuoteState::Single)
                        };
                        for candidate in &mut stack {
                            if matches!(candidate.quote_context, BraceQuoteContext::Unquoted) {
                                candidate.saw_quote_boundary = true;
                            }
                        }
                    }
                    '"' => {
                        quote_state = Some(QuoteState::Double);
                        for candidate in &mut stack {
                            if matches!(candidate.quote_context, BraceQuoteContext::Unquoted) {
                                candidate.saw_quote_boundary = true;
                            }
                        }
                    }
                    _ => {}
                },
                Some(QuoteState::Single) => {
                    if ch == '\'' {
                        quote_state = None;
                        for candidate in &mut stack {
                            if matches!(candidate.quote_context, BraceQuoteContext::Unquoted) {
                                candidate.saw_quote_boundary = true;
                            }
                        }
                    }
                }
                Some(QuoteState::AnsiSingle) => {
                    if ch == '\'' {
                        quote_state = None;
                        for candidate in &mut stack {
                            if matches!(candidate.quote_context, BraceQuoteContext::Unquoted) {
                                candidate.saw_quote_boundary = true;
                            }
                        }
                    }
                }
                Some(QuoteState::Double) => {
                    if ch == '"' {
                        quote_state = None;
                        for candidate in &mut stack {
                            if matches!(candidate.quote_context, BraceQuoteContext::Unquoted) {
                                candidate.saw_quote_boundary = true;
                            }
                        }
                    }
                }
            }

            match ch {
                '{' => {
                    mark_brace_ccl_content(&mut stack, quote_state);
                    stack.push(Candidate {
                        start: chars[index].1,
                        quote_context: current_quote_context,
                        has_comma: false,
                        has_dot_dot: false,
                        saw_unquoted_whitespace: false,
                        has_brace_ccl_content: false,
                        saw_quote_boundary: false,
                        prev_char: Some(ch),
                    });
                    index += 1;
                    continue;
                }
                '}' => {
                    if let Some(candidate_index) = last_active_candidate_index(&stack, quote_state)
                    {
                        let candidate = stack.remove(candidate_index);
                        let span = brace_span(candidate.start, chars[index]);
                        let kind = Self::classify_brace_construct_kind(
                            candidate.quote_context,
                            brace_ccl_enabled,
                            candidate.has_comma,
                            candidate.has_dot_dot,
                            candidate.saw_unquoted_whitespace,
                            candidate.has_brace_ccl_content,
                        );
                        if !matches!(kind, BraceSyntaxKind::Literal)
                            || !candidate.saw_quote_boundary
                        {
                            out.push(BraceSyntax {
                                kind,
                                span,
                                quote_context: candidate.quote_context,
                            });
                        }
                    }
                }
                _ => {
                    let is_quote_syntax = match quote_state {
                        None => {
                            matches!(ch, '\'' | '"')
                                || (ch == '$'
                                    && chars.get(index + 1).is_some_and(|(next, _)| *next == '\''))
                        }
                        Some(QuoteState::Single | QuoteState::AnsiSingle) => ch == '\'',
                        Some(QuoteState::Double) => ch == '"',
                    };
                    if ch != '\0' && !is_quote_syntax {
                        mark_brace_ccl_content(&mut stack, quote_state);
                    }
                    if let Some(candidate_index) = last_active_candidate_index(&stack, quote_state)
                    {
                        let candidate = &mut stack[candidate_index];
                        let counts_as_top_level =
                            quote_context_is_active(candidate.quote_context, quote_state);

                        if counts_as_top_level {
                            match ch {
                                ',' => candidate.has_comma = true,
                                '.' if candidate.prev_char == Some('.') => {
                                    candidate.has_dot_dot = true;
                                }
                                c if matches!(
                                    candidate.quote_context,
                                    BraceQuoteContext::Unquoted
                                ) && quote_state.is_none()
                                    && c.is_whitespace() =>
                                {
                                    candidate.saw_unquoted_whitespace = true;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

            if let Some(candidate_index) = last_active_candidate_index(&stack, quote_state) {
                stack[candidate_index].prev_char = Some(ch);
            }

            index += 1;
        }
    }

    fn collect_brace_syntax_from_pattern(
        &self,
        pattern: &Pattern,
        quote_context: BraceQuoteContext,
        brace_ccl_enabled: bool,
        out: &mut Vec<BraceSyntax>,
    ) {
        for (part, span) in pattern.parts_with_spans() {
            match part {
                PatternPart::Literal(text) => Self::scan_brace_syntax_text(
                    text.as_str(self.input, span),
                    span.start,
                    quote_context,
                    brace_ccl_enabled,
                    out,
                ),
                PatternPart::CharClass(text) => Self::scan_brace_syntax_text(
                    text.slice(self.input),
                    text.span().start,
                    quote_context,
                    brace_ccl_enabled,
                    out,
                ),
                PatternPart::Group { patterns, .. } => {
                    for pattern in patterns {
                        self.collect_brace_syntax_from_pattern(
                            pattern,
                            quote_context,
                            brace_ccl_enabled,
                            out,
                        );
                    }
                }
                PatternPart::Word(word) => {
                    self.collect_brace_syntax_from_parts(
                        &word.parts,
                        quote_context,
                        brace_ccl_enabled,
                        out,
                    );
                }
                PatternPart::AnyString | PatternPart::AnyChar => {}
            }
        }
    }

    fn collect_brace_syntax_from_zsh_qualified_glob(
        &self,
        glob: &ZshQualifiedGlob,
        quote_context: BraceQuoteContext,
        brace_ccl_enabled: bool,
        out: &mut Vec<BraceSyntax>,
    ) {
        for segment in &glob.segments {
            if let ZshGlobSegment::Pattern(pattern) = segment {
                self.collect_brace_syntax_from_pattern(
                    pattern,
                    quote_context,
                    brace_ccl_enabled,
                    out,
                );
            }
        }
    }

    fn scan_brace_syntax_text(
        text: &str,
        base: Position,
        quote_context: BraceQuoteContext,
        brace_ccl_enabled: bool,
        out: &mut Vec<BraceSyntax>,
    ) {
        if memchr(b'{', text.as_bytes()).is_none() {
            return;
        }

        #[derive(Clone, Copy)]
        struct ScanFrame<'a> {
            text: &'a str,
            index: usize,
            position: Position,
        }

        let mut work = SmallVec::<[ScanFrame<'_>; 2]>::new();
        work.push(ScanFrame {
            text,
            index: 0,
            position: base,
        });

        while let Some(mut frame) = work.pop() {
            let bytes = frame.text.as_bytes();

            while frame.index < bytes.len() {
                let next_special = if matches!(quote_context, BraceQuoteContext::Unquoted) {
                    memchr2(b'{', b'\\', &bytes[frame.index..])
                        .map(|relative| frame.index + relative)
                } else {
                    memchr(b'{', &bytes[frame.index..]).map(|relative| frame.index + relative)
                };

                let Some(next_index) = next_special else {
                    break;
                };

                if next_index > frame.index {
                    frame.position = frame
                        .position
                        .advanced_by(&frame.text[frame.index..next_index]);
                    frame.index = next_index;
                }

                if matches!(quote_context, BraceQuoteContext::Unquoted)
                    && bytes[frame.index] == b'\\'
                {
                    let escaped_start = frame.index;
                    frame.index += 1;
                    if let Some(next) = frame.text[frame.index..].chars().next() {
                        frame.index += next.len_utf8();
                    }
                    frame.position = frame
                        .position
                        .advanced_by(&frame.text[escaped_start..frame.index]);
                    continue;
                }

                let brace_start = frame.position;
                if let Some(len) =
                    Self::template_placeholder_len(frame.text, frame.index, quote_context)
                {
                    let brace_end =
                        brace_start.advanced_by(&frame.text[frame.index..frame.index + len]);
                    out.push(BraceSyntax {
                        kind: BraceSyntaxKind::TemplatePlaceholder,
                        span: Span::from_positions(brace_start, brace_end),
                        quote_context,
                    });
                    frame.position = brace_end;
                    frame.index += len;
                    continue;
                }

                if let Some((len, kind)) = Self::brace_construct_len(
                    frame.text,
                    frame.index,
                    quote_context,
                    brace_ccl_enabled,
                ) {
                    let brace_end =
                        brace_start.advanced_by(&frame.text[frame.index..frame.index + len]);
                    out.push(BraceSyntax {
                        kind,
                        span: Span::from_positions(brace_start, brace_end),
                        quote_context,
                    });

                    frame.position = brace_end;
                    frame.index += len;

                    if len > 2 {
                        let inner_start = frame.index - len + '{'.len_utf8();
                        let inner_end = frame.index - '}'.len_utf8();
                        if inner_start < inner_end {
                            let inner_base = brace_start.advanced_by("{");
                            work.push(frame);
                            work.push(ScanFrame {
                                text: &frame.text[inner_start..inner_end],
                                index: 0,
                                position: inner_base,
                            });
                            break;
                        }
                    }

                    continue;
                }

                frame.position.advance('{');
                frame.index += '{'.len_utf8();
            }
        }
    }

    pub(super) fn text_position(base: Position, text: &str, offset: usize) -> Position {
        base.advanced_by(&text[..offset])
    }

    fn template_placeholder_len(
        text: &str,
        start: usize,
        quote_context: BraceQuoteContext,
    ) -> Option<usize> {
        text.get(start..).filter(|rest| rest.starts_with("{{"))?;

        let mut index = start + "{{".len();
        let mut depth = 1usize;

        while index < text.len() {
            if matches!(quote_context, BraceQuoteContext::Unquoted)
                && text[index..].starts_with('\\')
            {
                index += 1;
                if let Some(next) = text[index..].chars().next() {
                    index += next.len_utf8();
                }
                continue;
            }

            if text[index..].starts_with("{{") {
                depth += 1;
                index += "{{".len();
                continue;
            }

            if text[index..].starts_with("}}") {
                depth -= 1;
                index += "}}".len();
                if depth == 0 {
                    return Some(index - start);
                }
                continue;
            }

            index += text[index..].chars().next()?.len_utf8();
        }

        None
    }

    fn brace_construct_len(
        text: &str,
        start: usize,
        quote_context: BraceQuoteContext,
        brace_ccl_enabled: bool,
    ) -> Option<(usize, BraceSyntaxKind)> {
        text.get(start..).filter(|rest| rest.starts_with('{'))?;

        #[derive(Clone, Copy, PartialEq, Eq)]
        enum QuoteState {
            Single,
            Double,
        }

        let mut index = start + '{'.len_utf8();
        let mut depth = 1usize;
        let mut has_comma = false;
        let mut has_dot_dot = false;
        let mut saw_unquoted_whitespace = false;
        let mut has_brace_ccl_content = false;
        let mut prev_char = None;
        let mut quote_state = None;

        while index < text.len() {
            if matches!(quote_context, BraceQuoteContext::Unquoted)
                && quote_state.is_none()
                && text[index..].starts_with('\\')
            {
                index += 1;
                if let Some(next) = text[index..].chars().next() {
                    if depth == 1 {
                        has_brace_ccl_content = true;
                    }
                    index += next.len_utf8();
                }
                prev_char = None;
                continue;
            }

            let ch = text[index..].chars().next()?;
            index += ch.len_utf8();

            if matches!(quote_context, BraceQuoteContext::Unquoted) {
                match quote_state {
                    None => match ch {
                        '\'' => {
                            quote_state = Some(QuoteState::Single);
                            prev_char = None;
                            continue;
                        }
                        '"' => {
                            quote_state = Some(QuoteState::Double);
                            prev_char = None;
                            continue;
                        }
                        '$' if text[index..].starts_with('\'') => {}
                        '{' => {
                            has_brace_ccl_content = true;
                            depth += 1;
                        }
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                let kind = Self::classify_brace_construct_kind(
                                    quote_context,
                                    brace_ccl_enabled,
                                    has_comma,
                                    has_dot_dot,
                                    saw_unquoted_whitespace,
                                    has_brace_ccl_content,
                                );
                                return Some((index - start, kind));
                            }
                        }
                        ',' if depth == 1 => has_comma = true,
                        '.' if depth == 1 && prev_char == Some('.') => has_dot_dot = true,
                        c if c.is_whitespace() => saw_unquoted_whitespace = true,
                        _ => has_brace_ccl_content = true,
                    },
                    Some(QuoteState::Single) => {
                        if ch == '\'' {
                            quote_state = None;
                        } else {
                            has_brace_ccl_content = true;
                        }
                    }
                    Some(QuoteState::Double) => match ch {
                        '\\' => {
                            if let Some(next) = text[index..].chars().next() {
                                has_brace_ccl_content = true;
                                index += next.len_utf8();
                            }
                            prev_char = None;
                            continue;
                        }
                        '"' => quote_state = None,
                        _ => has_brace_ccl_content = true,
                    },
                }
            } else {
                match ch {
                    '{' => {
                        has_brace_ccl_content = true;
                        depth += 1;
                    }
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            let kind = Self::classify_brace_construct_kind(
                                quote_context,
                                brace_ccl_enabled,
                                has_comma,
                                has_dot_dot,
                                false,
                                has_brace_ccl_content,
                            );
                            return Some((index - start, kind));
                        }
                    }
                    ',' if depth == 1 => has_comma = true,
                    '.' if depth == 1 && prev_char == Some('.') => has_dot_dot = true,
                    _ => has_brace_ccl_content = true,
                }
            }

            prev_char = Some(ch);
        }

        None
    }

    fn classify_brace_construct_kind(
        quote_context: BraceQuoteContext,
        brace_ccl_enabled: bool,
        has_comma: bool,
        has_dot_dot: bool,
        saw_unquoted_whitespace: bool,
        has_brace_ccl_content: bool,
    ) -> BraceSyntaxKind {
        if matches!(quote_context, BraceQuoteContext::Unquoted) && saw_unquoted_whitespace {
            BraceSyntaxKind::Literal
        } else if has_comma {
            BraceSyntaxKind::Expansion(BraceExpansionKind::CommaList)
        } else if has_dot_dot {
            BraceSyntaxKind::Expansion(BraceExpansionKind::Sequence)
        } else if brace_ccl_enabled && has_brace_ccl_content {
            BraceSyntaxKind::Expansion(BraceExpansionKind::CharacterClass)
        } else {
            BraceSyntaxKind::Literal
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct WordSurfaceInterest {
    brace_syntax: bool,
    escaped_parameter_templates: bool,
    all_elements_array_expansions: bool,
    zsh_short_positional_at: bool,
}

impl WordSurfaceInterest {
    const fn is_empty(self) -> bool {
        !(self.brace_syntax
            || self.escaped_parameter_templates
            || self.all_elements_array_expansions
            || self.zsh_short_positional_at)
    }
}

fn has_escaped_parameter_template(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut start = 0usize;
    while let Some(relative) = memchr(b'\\', &bytes[start..]) {
        let index = start + relative;
        if bytes.get(index + 1) == Some(&b'$') && bytes.get(index + 2) == Some(&b'{') {
            return true;
        }
        start = index + 1;
    }
    false
}

fn has_zsh_short_positional_at(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut start = 0usize;
    while let Some(relative) = memchr(b'@', &bytes[start..]) {
        let index = start + relative;
        if bytes.get(index + 1) == Some(&b'[') {
            return true;
        }
        start = index + 1;
    }
    false
}

fn span_start_inside_escaped_parameter_template(
    span: Span,
    templates: &[EscapedParameterTemplateSyntax],
) -> bool {
    templates.iter().any(|template| {
        template.body_span.start.offset <= span.start.offset
            && span.start.offset < template.body_span.end.offset
    })
}

fn escaped_parameter_template_end(text: &str, dollar_offset: usize) -> Option<usize> {
    if dollar_offset >= text.len() || !text[dollar_offset..].starts_with("${") {
        return None;
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum QuoteState {
        None,
        Single,
        Double,
    }

    let bytes = text.as_bytes();
    let mut index = dollar_offset + "${".len();
    let mut depth = 1usize;
    let mut quote_state = QuoteState::None;

    while index < bytes.len() {
        let byte = bytes[index];
        match quote_state {
            QuoteState::Single => {
                if byte == b'\'' {
                    quote_state = QuoteState::None;
                }
                index += 1;
                continue;
            }
            QuoteState::Double => {
                if byte == b'\\' {
                    index += usize::from(index + 1 < bytes.len()) + 1;
                    continue;
                }
                if byte == b'"' {
                    quote_state = QuoteState::None;
                }
                index += 1;
                continue;
            }
            QuoteState::None => {}
        }

        match byte {
            b'\\' => {
                index += usize::from(index + 1 < bytes.len()) + 1;
            }
            b'\'' => {
                quote_state = QuoteState::Single;
                index += 1;
            }
            b'"' => {
                quote_state = QuoteState::Double;
                index += 1;
            }
            b'$' if bytes.get(index + 1) == Some(&b'{') => {
                depth += 1;
                index += "${".len();
            }
            b'}' => {
                depth -= 1;
                index += '}'.len_utf8();
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index = advance_shell_char(text, index),
        }
    }

    None
}

fn advance_shell_char(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map_or(text.len(), |ch| index + ch.len_utf8())
}

fn offset_is_backslash_escaped(offset: usize, source: &str) -> bool {
    let mut count = 0usize;
    let mut cursor = offset;
    while let Some(prev) = cursor.checked_sub(1) {
        if source.as_bytes().get(prev) != Some(&b'\\') {
            break;
        }
        count += 1;
        cursor = prev;
    }
    count % 2 == 1
}

fn zsh_short_positional_at_suffix(text: &str) -> Option<(usize, ZshShortPositionalAtKind)> {
    let rest = text.strip_prefix('[')?;
    let close = rest.find(']')?;
    let inner = &rest[..close];
    if matches!(inner, "@" | "*") {
        return None;
    }
    let kind = if inner.contains(',') {
        ZshShortPositionalAtKind::Range
    } else {
        ZshShortPositionalAtKind::IndexedSubscript
    };
    Some((close + 2, kind))
}

pub(super) fn all_elements_surface_for_part(
    parts: &[WordPartNode],
    index: usize,
    part: &WordPart,
    source: &str,
    dialect: ShellDialect,
) -> Option<(AllElementsArrayExpansionKind, bool)> {
    match part {
        WordPart::Variable(name) => match name.as_str() {
            "@" if !part_has_zsh_short_positional_subscript(parts, index, source, dialect) => {
                Some((AllElementsArrayExpansionKind::PositionalAt, true))
            }
            "*" => Some((AllElementsArrayExpansionKind::PositionalStar, true)),
            _ => None,
        },
        WordPart::ArrayAccess(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::Substring { reference, .. }
        | WordPart::ArraySlice { reference, .. }
        | WordPart::Transformation { reference, .. } => {
            all_elements_kind_for_var_ref(reference, dialect).map(|kind| (kind, true))
        }
        WordPart::ParameterExpansion {
            reference,
            operator,
            ..
        } => all_elements_kind_for_var_ref(reference, dialect).map(|kind| {
            (
                kind,
                !matches!(operator.as_ref(), ParameterOp::UseReplacement),
            )
        }),
        WordPart::IndirectExpansion {
            reference,
            operator,
            ..
        } => all_elements_kind_for_var_ref(reference, dialect).map(|kind| {
            (
                kind,
                !operator
                    .as_deref()
                    .is_some_and(|operator| matches!(operator, ParameterOp::UseReplacement)),
            )
        }),
        WordPart::PrefixMatch {
            kind: PrefixMatchKind::At,
            ..
        } => Some((AllElementsArrayExpansionKind::PositionalAt, false)),
        WordPart::PrefixMatch {
            kind: PrefixMatchKind::Star,
            ..
        } => None,
        WordPart::Parameter(parameter) => {
            all_elements_kind_for_parameter(parameter, dialect).map(|kind| {
                (
                    kind,
                    direct_all_elements_kind_for_parameter(parameter, dialect).is_some(),
                )
            })
        }
        WordPart::Literal(_)
        | WordPart::ZshQualifiedGlob(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::DoubleQuoted { .. }
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayLength(_)
        | WordPart::ProcessSubstitution { .. } => None,
    }
}

fn all_elements_kind_for_parameter(
    parameter: &ParameterExpansion,
    dialect: ShellDialect,
) -> Option<AllElementsArrayExpansionKind> {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Slice { reference, .. }
            | BourneParameterExpansion::Operation { reference, .. }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                all_elements_kind_for_var_ref(reference, dialect)
            }
            BourneParameterExpansion::PrefixMatch { kind, .. } => match kind {
                PrefixMatchKind::At => Some(AllElementsArrayExpansionKind::PositionalAt),
                PrefixMatchKind::Star => None,
            },
            BourneParameterExpansion::Length { .. } | BourneParameterExpansion::Indirect { .. } => {
                None
            }
        },
        // zsh-typed expansions carry additional modifier semantics. Those
        // can join, filter, or otherwise reshape the target, so we keep the
        // array-collapse facts conservative and only classify Bourne-typed
        // surfaces as direct all-elements expansions here.
        ParameterExpansionSyntax::Zsh(_) => None,
    }
}

fn direct_all_elements_kind_for_parameter(
    parameter: &ParameterExpansion,
    dialect: ShellDialect,
) -> Option<AllElementsArrayExpansionKind> {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Slice { reference, .. }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                all_elements_kind_for_var_ref(reference, dialect)
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                ..
            } => (!matches!(operator.as_ref(), ParameterOp::UseReplacement))
                .then(|| all_elements_kind_for_var_ref(reference, dialect))
                .flatten(),
            BourneParameterExpansion::PrefixMatch { .. } => None,
            BourneParameterExpansion::Length { .. } | BourneParameterExpansion::Indirect { .. } => {
                None
            }
        },
        ParameterExpansionSyntax::Zsh(_) => None,
    }
}

fn all_elements_kind_for_var_ref(
    reference: &VarRef,
    dialect: ShellDialect,
) -> Option<AllElementsArrayExpansionKind> {
    if reference.name.as_str() == "@"
        && !(dialect == ShellDialect::Zsh
            && reference
                .subscript
                .as_deref()
                .is_some_and(|subscript| subscript.selector().is_none()))
    {
        return Some(AllElementsArrayExpansionKind::PositionalAt);
    }
    if reference.name.as_str() == "*" {
        return Some(AllElementsArrayExpansionKind::PositionalStar);
    }

    match reference
        .subscript
        .as_deref()
        .and_then(|subscript| subscript.selector())
    {
        Some(SubscriptSelector::At) => Some(AllElementsArrayExpansionKind::SelectorAt),
        Some(SubscriptSelector::Star) => Some(AllElementsArrayExpansionKind::SelectorStar),
        None => None,
    }
}

fn part_has_zsh_short_positional_subscript(
    parts: &[WordPartNode],
    index: usize,
    source: &str,
    dialect: ShellDialect,
) -> bool {
    dialect == ShellDialect::Zsh
        && parts.get(index).is_some_and(
            |part| matches!(&part.kind, WordPart::Variable(name) if name.as_str() == "@"),
        )
        && parts.get(index + 1).is_some_and(|next| {
            matches!(
                &next.kind,
                WordPart::Literal(text)
                    if zsh_short_positional_at_suffix(text.as_str(source, next.span)).is_some()
            )
        })
}

fn nested_all_elements_array_expansion_in_parameter(
    parameter: &ParameterExpansion,
    span: Span,
    source: &str,
    dialect: ShellDialect,
) -> Option<(Span, AllElementsArrayExpansionKind)> {
    if !parameter_might_have_nested_all_elements_array_expansion(parameter, span, source, dialect) {
        return None;
    }

    first_all_elements_array_expansion_in_text(span, source, dialect)
}

fn first_all_elements_array_expansion_in_text(
    span: Span,
    source: &str,
    dialect: ShellDialect,
) -> Option<(Span, AllElementsArrayExpansionKind)> {
    let text = span.slice(source);
    if !text.contains('$') {
        return None;
    }

    let base_offset = span.start.offset;
    let mut search_from = 0usize;

    while let Some(found) = text[search_from..].find('$') {
        let relative_start = search_from + found;
        let absolute_start = base_offset + relative_start;
        if offset_is_backslash_escaped(absolute_start, source) {
            search_from = relative_start + 1;
            continue;
        }

        let start = span.start.advanced_by(&text[..relative_start]);
        let remainder = &source[absolute_start..];

        if let Some(after_dollar) = remainder.strip_prefix('$') {
            if after_dollar.starts_with('@') {
                if source_starts_with_zsh_short_positional_suffix(
                    &after_dollar['@'.len_utf8()..],
                    dialect,
                ) {
                    search_from = relative_start + 1;
                    continue;
                }
                let end = start.advanced_by("$@");
                return Some((
                    Span::from_positions(start, end),
                    AllElementsArrayExpansionKind::PositionalAt,
                ));
            }
            if after_dollar.starts_with('*') {
                let end = start.advanced_by("$*");
                return Some((
                    Span::from_positions(start, end),
                    AllElementsArrayExpansionKind::PositionalStar,
                ));
            }
        }

        if remainder.starts_with("${")
            && let Some(relative_end) = remainder.find('}')
        {
            let candidate = &remainder[..=relative_end];
            if let Some(kind) = candidate_all_elements_array_expansion_kind(candidate, dialect) {
                let end = start.advanced_by(candidate);
                return Some((Span::from_positions(start, end), kind));
            }
        }

        search_from = relative_start + 1;
    }

    None
}

fn parameter_might_have_nested_all_elements_array_expansion(
    parameter: &ParameterExpansion,
    span: Span,
    source: &str,
    dialect: ShellDialect,
) -> bool {
    if !span.slice(source).contains(['@', '*']) {
        return false;
    }

    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Slice { reference, .. }
            | BourneParameterExpansion::Operation { reference, .. }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                !(dialect == ShellDialect::Zsh
                    && reference.name.as_str() == "@"
                    && reference
                        .subscript
                        .as_deref()
                        .is_some_and(|subscript| subscript.selector().is_none()))
            }
            BourneParameterExpansion::Length { .. } | BourneParameterExpansion::Indirect { .. } => {
                false
            }
            BourneParameterExpansion::PrefixMatch { .. } => true,
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Reference(reference) => {
                !(dialect == ShellDialect::Zsh
                    && reference.name.as_str() == "@"
                    && reference
                        .subscript
                        .as_deref()
                        .is_some_and(|subscript| subscript.selector().is_none()))
            }
            ZshExpansionTarget::Nested(_)
            | ZshExpansionTarget::Word(_)
            | ZshExpansionTarget::Empty => true,
        },
    }
}

fn candidate_all_elements_array_expansion_kind(
    candidate: &str,
    dialect: ShellDialect,
) -> Option<AllElementsArrayExpansionKind> {
    let mut inner = candidate
        .strip_prefix("${")
        .and_then(|text| text.strip_suffix('}'))?;

    if let Some(stripped) = inner.strip_prefix('!') {
        inner = stripped;
    }

    if let Some(stripped) = inner.strip_prefix('@') {
        let suffix = strip_special_positional_suffix(stripped, dialect)?;
        if suffix.starts_with('+') || suffix.starts_with(":+") {
            return Some(AllElementsArrayExpansionKind::PositionalAt);
        }
        return Some(AllElementsArrayExpansionKind::PositionalAt);
    }
    if inner.starts_with('*') {
        return Some(AllElementsArrayExpansionKind::PositionalStar);
    }

    let first = inner.as_bytes().first().copied()?;
    if !is_name_start(first) {
        return None;
    }

    let bytes = inner.as_bytes();
    let mut index = 1usize;
    while index < bytes.len() && is_name_continue(bytes[index]) {
        index += 1;
    }

    let suffix = &inner[index..];
    if suffix.starts_with("[@]") {
        return Some(AllElementsArrayExpansionKind::SelectorAt);
    }
    if suffix.starts_with("[*]") {
        return Some(AllElementsArrayExpansionKind::SelectorStar);
    }
    if candidate.starts_with("${!") && suffix.starts_with('@') {
        return Some(AllElementsArrayExpansionKind::PositionalAt);
    }
    None
}

fn candidate_direct_all_elements_array_expansion_kind(
    candidate: &str,
    dialect: ShellDialect,
) -> Option<AllElementsArrayExpansionKind> {
    let kind = candidate_all_elements_array_expansion_kind(candidate, dialect)?;
    let mut inner = candidate
        .strip_prefix("${")
        .and_then(|text| text.strip_suffix('}'))?;

    if let Some(stripped) = inner.strip_prefix('!') {
        inner = stripped;
    }

    if inner.starts_with('@') {
        let suffix = strip_special_positional_suffix(&inner['@'.len_utf8()..], dialect)?;
        if suffix.starts_with('+') || suffix.starts_with(":+") {
            return None;
        }
        return Some(kind);
    }

    let first = inner.as_bytes().first().copied()?;
    if !is_name_start(first) {
        return None;
    }

    let bytes = inner.as_bytes();
    let mut index = 1usize;
    while index < bytes.len() && is_name_continue(bytes[index]) {
        index += 1;
    }

    let suffix = &inner[index..];
    let suffix = if let Some(stripped) = suffix.strip_prefix("[@]") {
        stripped
    } else {
        return None;
    };

    if suffix.starts_with('+') || suffix.starts_with(":+") {
        return None;
    }

    Some(kind)
}

fn strip_special_positional_suffix<'a>(suffix: &'a str, dialect: ShellDialect) -> Option<&'a str> {
    if let Some(stripped) = strip_zsh_positional_selector_suffix(suffix) {
        return Some(stripped);
    }
    if dialect == ShellDialect::Zsh && suffix.starts_with('[') {
        return None;
    }
    Some(suffix)
}

fn strip_zsh_positional_selector_suffix(suffix: &str) -> Option<&str> {
    let rest = suffix.strip_prefix('[')?;
    let close = rest.find(']')?;
    match &rest[..close] {
        "@" | "*" => Some(&rest[close + 1..]),
        _ => None,
    }
}

fn source_starts_with_zsh_short_positional_suffix(text: &str, dialect: ShellDialect) -> bool {
    if dialect != ShellDialect::Zsh {
        return false;
    }
    zsh_short_positional_at_suffix(text).is_some()
}

fn is_name_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_name_continue(byte: u8) -> bool {
    is_name_start(byte) || byte.is_ascii_digit()
}
