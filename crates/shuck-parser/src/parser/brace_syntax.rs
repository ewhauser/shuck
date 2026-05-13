use super::*;

impl<'a> Parser<'a> {
    pub(super) fn brace_syntax_from_parts(
        &self,
        parts: &[WordPartNode],
        offset: usize,
    ) -> Vec<BraceSyntax> {
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
        brace_syntax.sort_by_key(|brace| (brace.span.start.offset, brace.span.end.offset));
        brace_syntax.dedup_by_key(|brace| {
            (
                brace.span.start.offset,
                brace.span.end.offset,
                brace.quote_context,
                brace.kind,
            )
        });
        brace_syntax
    }

    pub(super) fn collect_brace_syntax_from_parts(
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

    pub(super) fn needs_cross_part_brace_scan(&self, parts: &[WordPartNode]) -> bool {
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

    pub(super) fn collect_brace_scan_chars_from_parts(
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

    pub(super) fn collect_brace_scan_chars_from_double_quoted_part(
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

    pub(super) fn push_brace_scan_text(
        text: &str,
        start: Position,
        out: &mut Vec<(char, Position)>,
    ) {
        let mut position = start;
        for ch in text.chars() {
            out.push((ch, position));
            position.advance(ch);
        }
    }

    pub(super) fn push_brace_scan_boundary(position: Position, out: &mut Vec<(char, Position)>) {
        out.push(('\0', position));
    }

    pub(super) fn scan_brace_syntax_chars(
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

    pub(super) fn collect_brace_syntax_from_pattern(
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
                PatternPart::Word(word) => self.collect_brace_syntax_from_parts(
                    &word.parts,
                    quote_context,
                    brace_ccl_enabled,
                    out,
                ),
                PatternPart::AnyString | PatternPart::AnyChar => {}
            }
        }
    }

    pub(super) fn collect_brace_syntax_from_zsh_qualified_glob(
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

    pub(super) fn scan_brace_syntax_text(
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

    pub(super) fn template_placeholder_len(
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

    pub(super) fn brace_construct_len(
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

    pub(super) fn classify_brace_construct_kind(
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

    pub(super) fn maybe_parse_zsh_qualified_glob_word(
        &mut self,
        text: &str,
        span: Span,
        source_backed: bool,
    ) -> Option<Word> {
        let features = self.zsh_glob_parse_features_at(span.start.offset);
        if !self.zsh_glob_word_parsing_enabled_at(span.start.offset)
            || text.is_empty()
            || text.contains('=')
            || text.contains(['\x00', '\\', '\'', '"', '$', '`'])
            || text.chars().any(char::is_whitespace)
        {
            return None;
        }

        let (segments, qualifiers, saw_glob_syntax) =
            self.parse_zsh_qualified_glob_segments(text, span, source_backed, features)?;
        if !saw_glob_syntax {
            return None;
        }

        Some(self.word_with_parts(
            vec![WordPartNode::new(
                WordPart::ZshQualifiedGlob(ZshQualifiedGlob {
                    span,
                    segments,
                    qualifiers,
                }),
                span,
            )],
            span,
        ))
    }

    pub(super) fn parse_zsh_qualified_glob_segments(
        &mut self,
        text: &str,
        span: Span,
        source_backed: bool,
        features: ZshGlobParseFeatures,
    ) -> Option<(Vec<ZshGlobSegment>, Option<ZshGlobQualifierGroup>, bool)> {
        let mut segments = Vec::new();
        let mut qualifiers = None;
        let mut saw_glob_syntax = false;
        let mut pattern_start = 0usize;
        let mut index = 0usize;

        while index < text.len() {
            if features.extended_glob && text[index..].starts_with("(#") {
                if let Some((len, control)) =
                    self.parse_zsh_inline_glob_control(text, span.start, index)
                {
                    self.push_zsh_pattern_segment(
                        &mut segments,
                        text,
                        span.start,
                        (pattern_start, index),
                        source_backed,
                        features,
                    );
                    segments.push(ZshGlobSegment::InlineControl(control));
                    saw_glob_syntax = true;
                    index += len;
                    pattern_start = index;
                    continue;
                }

                let suffix_start = Self::text_position(span.start, text, index);
                if let Some(group) = self.parse_zsh_terminal_glob_qualifier_group(
                    &text[index..],
                    suffix_start,
                    source_backed,
                ) {
                    self.push_zsh_pattern_segment(
                        &mut segments,
                        text,
                        span.start,
                        (pattern_start, index),
                        source_backed,
                        features,
                    );
                    qualifiers = Some(group);
                    saw_glob_syntax = true;
                    index = text.len();
                    pattern_start = index;
                    break;
                }
            }

            if features.classic_qualifiers && text[index..].starts_with('(') {
                let suffix_start = Self::text_position(span.start, text, index);
                if let Some(group) = self.parse_zsh_terminal_glob_qualifier_group(
                    &text[index..],
                    suffix_start,
                    source_backed,
                ) && matches!(group.kind, ZshGlobQualifierKind::Classic)
                {
                    self.push_zsh_pattern_segment(
                        &mut segments,
                        text,
                        span.start,
                        (pattern_start, index),
                        source_backed,
                        features,
                    );
                    qualifiers = Some(group);
                    saw_glob_syntax = true;
                    index = text.len();
                    pattern_start = index;
                    break;
                }
            }

            index += text[index..].chars().next()?.len_utf8();
        }

        saw_glob_syntax |= self.push_zsh_pattern_segment(
            &mut segments,
            text,
            span.start,
            (pattern_start, text.len()),
            source_backed,
            features,
        );

        segments
            .iter()
            .any(|segment| matches!(segment, ZshGlobSegment::Pattern(_)))
            .then_some((segments, qualifiers, saw_glob_syntax))
    }

    pub(super) fn push_zsh_pattern_segment(
        &mut self,
        segments: &mut Vec<ZshGlobSegment>,
        text: &str,
        base: Position,
        bounds: (usize, usize),
        source_backed: bool,
        features: ZshGlobParseFeatures,
    ) -> bool {
        let (start, end) = bounds;
        if start >= end {
            return false;
        }

        let span = Span::from_positions(
            Self::text_position(base, text, start),
            Self::text_position(base, text, end),
        );
        let pattern_word =
            self.decode_word_text(&text[start..end], span, span.start, source_backed);
        let pattern = self.pattern_from_word(&pattern_word);
        let saw_glob_syntax = self.pattern_has_glob_syntax_with_features(&pattern, features);
        segments.push(ZshGlobSegment::Pattern(pattern));
        saw_glob_syntax
    }

    pub(super) fn parse_zsh_inline_glob_control(
        &self,
        text: &str,
        base: Position,
        start: usize,
    ) -> Option<(usize, ZshInlineGlobControl)> {
        let (len, control) = if text[start..].starts_with("(#i)") {
            (
                "(#i)".len(),
                ZshInlineGlobControl::CaseInsensitive {
                    span: Span::from_positions(
                        Self::text_position(base, text, start),
                        Self::text_position(base, text, start + "(#i)".len()),
                    ),
                },
            )
        } else if text[start..].starts_with("(#b)") {
            (
                "(#b)".len(),
                ZshInlineGlobControl::Backreferences {
                    span: Span::from_positions(
                        Self::text_position(base, text, start),
                        Self::text_position(base, text, start + "(#b)".len()),
                    ),
                },
            )
        } else if text[start..].starts_with("(#s)") {
            (
                "(#s)".len(),
                ZshInlineGlobControl::StartAnchor {
                    span: Span::from_positions(
                        Self::text_position(base, text, start),
                        Self::text_position(base, text, start + "(#s)".len()),
                    ),
                },
            )
        } else if text[start..].starts_with("(#e)") {
            (
                "(#e)".len(),
                ZshInlineGlobControl::EndAnchor {
                    span: Span::from_positions(
                        Self::text_position(base, text, start),
                        Self::text_position(base, text, start + "(#e)".len()),
                    ),
                },
            )
        } else {
            return None;
        };

        Some((len, control))
    }

    pub(super) fn parse_zsh_terminal_glob_qualifier_group(
        &self,
        text: &str,
        base: Position,
        source_backed: bool,
    ) -> Option<ZshGlobQualifierGroup> {
        let (kind, prefix_len, inner) = if let Some(inner) = text
            .strip_prefix("(#q")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            (ZshGlobQualifierKind::HashQ, "(#q".len(), inner)
        } else {
            let inner = text.strip_prefix('(')?.strip_suffix(')')?;
            (ZshGlobQualifierKind::Classic, "(".len(), inner)
        };

        let fragments = self.parse_zsh_glob_qualifier_fragments(
            inner,
            Self::text_position(base, text, prefix_len),
            source_backed,
        )?;

        Some(ZshGlobQualifierGroup {
            span: Span::from_positions(base, Self::text_position(base, text, text.len())),
            kind,
            fragments,
        })
    }

    pub(super) fn parse_zsh_glob_qualifier_fragments(
        &self,
        text: &str,
        base: Position,
        source_backed: bool,
    ) -> Option<Vec<ZshGlobQualifier>> {
        let mut fragments = Vec::new();
        let mut index = 0;
        let mut saw_non_letter_fragment = false;

        while index < text.len() {
            let start = index;
            let ch = text[index..].chars().next()?;

            match ch {
                '^' => {
                    index += ch.len_utf8();
                    fragments.push(ZshGlobQualifier::Negation {
                        span: Span::from_positions(
                            Self::text_position(base, text, start),
                            Self::text_position(base, text, index),
                        ),
                    });
                    saw_non_letter_fragment = true;
                }
                '.' | '/' | '-' | 'A'..='Z' => {
                    index += ch.len_utf8();
                    fragments.push(ZshGlobQualifier::Flag {
                        name: ch,
                        span: Span::from_positions(
                            Self::text_position(base, text, start),
                            Self::text_position(base, text, index),
                        ),
                    });
                    saw_non_letter_fragment = true;
                }
                '[' => {
                    index += ch.len_utf8();
                    let number_start = index;
                    while matches!(text[index..].chars().next(), Some('0'..='9')) {
                        index += 1;
                    }
                    if number_start == index {
                        return None;
                    }

                    let start_text = self.zsh_glob_qualifier_source_text(
                        text,
                        base,
                        number_start,
                        index,
                        source_backed,
                    );
                    let end_text = if text[index..].starts_with(',') {
                        index += 1;
                        let range_start = index;
                        while matches!(text[index..].chars().next(), Some('0'..='9')) {
                            index += 1;
                        }
                        if range_start == index {
                            return None;
                        }
                        Some(self.zsh_glob_qualifier_source_text(
                            text,
                            base,
                            range_start,
                            index,
                            source_backed,
                        ))
                    } else {
                        None
                    };

                    if !text[index..].starts_with(']') {
                        return None;
                    }
                    index += "]".len();
                    fragments.push(ZshGlobQualifier::NumericArgument {
                        span: Span::from_positions(
                            Self::text_position(base, text, start),
                            Self::text_position(base, text, index),
                        ),
                        start: start_text,
                        end: end_text,
                    });
                    saw_non_letter_fragment = true;
                }
                'a'..='z' => {
                    index += ch.len_utf8();
                    while matches!(text[index..].chars().next(), Some('a'..='z' | 'A'..='Z')) {
                        index += 1;
                    }
                    if index - start <= 1 {
                        return None;
                    }
                    fragments.push(ZshGlobQualifier::LetterSequence {
                        text: self.zsh_glob_qualifier_source_text(
                            text,
                            base,
                            start,
                            index,
                            source_backed,
                        ),
                        span: Span::from_positions(
                            Self::text_position(base, text, start),
                            Self::text_position(base, text, index),
                        ),
                    });
                }
                _ => return None,
            }
        }

        (!fragments.is_empty() && saw_non_letter_fragment).then_some(fragments)
    }

    pub(super) fn zsh_glob_qualifier_source_text(
        &self,
        text: &str,
        base: Position,
        start: usize,
        end: usize,
        source_backed: bool,
    ) -> SourceText {
        let span = Span::from_positions(
            Self::text_position(base, text, start),
            Self::text_position(base, text, end),
        );
        if source_backed {
            SourceText::source(span)
        } else {
            SourceText::cooked(span, text[start..end].to_string())
        }
    }

    pub(super) fn pattern_has_glob_syntax_with_features(
        &self,
        pattern: &Pattern,
        features: ZshGlobParseFeatures,
    ) -> bool {
        pattern.parts.iter().any(|part| match &part.kind {
            PatternPart::AnyString | PatternPart::AnyChar | PatternPart::CharClass(_) => true,
            PatternPart::Group { .. } => true,
            PatternPart::Word(word) => Self::pattern_has_glob_word(word),
            PatternPart::Literal(text) => {
                Self::literal_text_has_zsh_glob_syntax(text.as_str(self.input, part.span), features)
            }
        })
    }

    pub(super) fn pattern_has_glob_word(word: &Word) -> bool {
        word.parts
            .iter()
            .any(|part| !matches!(part.kind, WordPart::Literal(_)))
    }

    pub(super) fn literal_text_has_zsh_glob_syntax(
        text: &str,
        features: ZshGlobParseFeatures,
    ) -> bool {
        if text.is_empty() {
            return false;
        }

        if features.extended_glob && text.contains("(#") {
            return true;
        }

        let bytes = text.as_bytes();
        let mut escaped = false;
        let mut bracket_depth = 0usize;
        let mut previous_char = None;
        let mut index = 0usize;

        while index < bytes.len() {
            let Some(ch) = text[index..].chars().next() else {
                break;
            };

            if escaped {
                escaped = false;
                previous_char = Some(ch);
                index += ch.len_utf8();
                continue;
            }

            if ch == '\\' {
                escaped = true;
                previous_char = Some(ch);
                index += ch.len_utf8();
                continue;
            }

            if bracket_depth == 0 {
                if features.extended_glob
                    && (ch == '~'
                        || ch == '#'
                        || (ch == '^'
                            && previous_char.is_none_or(|prev| prev == '(' || prev == '|')))
                {
                    return true;
                }

                if features.bare_groups
                    && ch == '<'
                    && Self::literal_text_has_numeric_range_suffix(&text[index..])
                {
                    return true;
                }
            }

            match ch {
                '[' => bracket_depth += 1,
                ']' if bracket_depth > 0 => bracket_depth -= 1,
                _ => {}
            }

            previous_char = Some(ch);
            index += ch.len_utf8();
        }

        false
    }

    pub(super) fn literal_text_has_numeric_range_suffix(text: &str) -> bool {
        let Some(rest) = text.strip_prefix('<') else {
            return false;
        };

        let mut saw_body = false;
        let mut saw_hyphen = false;
        for ch in rest.chars() {
            if ch == '>' {
                return saw_body && saw_hyphen;
            }
            if !matches!(ch, '0'..='9' | '-') {
                return false;
            }
            saw_hyphen |= ch == '-';
            saw_body = true;
        }

        false
    }
}
