#[derive(Debug, Clone)]
pub struct CaseItemFact {
    command_id: CommandId,
    case_span: Span,
    pattern_spans: Box<[Span]>,
    body_span: Span,
    first_body_stmt_span: Option<Span>,
    terminator: CaseTerminator,
    terminator_span: Option<Span>,
    suspicious_bracket_glob_spans: Box<[Span]>,
}

impl CaseItemFact {
    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn terminator(&self) -> CaseTerminator {
        self.terminator
    }

    pub fn terminator_span(&self) -> Option<Span> {
        self.terminator_span
    }

    pub fn case_span(&self) -> Span {
        self.case_span
    }

    pub fn pattern_spans(&self) -> &[Span] {
        &self.pattern_spans
    }

    pub fn last_pattern_span(&self) -> Option<Span> {
        self.pattern_spans.last().copied()
    }

    pub fn body_span(&self) -> Span {
        self.body_span
    }

    pub fn first_body_stmt_span(&self) -> Option<Span> {
        self.first_body_stmt_span
    }

    pub fn suspicious_bracket_glob_spans(&self) -> &[Span] {
        &self.suspicious_bracket_glob_spans
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CasePatternShadowFact {
    shadowing_pattern_span: Span,
    shadowed_pattern_span: Span,
}

impl CasePatternShadowFact {
    pub fn shadowing_pattern_span(&self) -> Span {
        self.shadowing_pattern_span
    }

    pub fn shadowed_pattern_span(&self) -> Span {
        self.shadowed_pattern_span
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetoptsOptionSpec {
    option: char,
    requires_argument: bool,
}

impl GetoptsOptionSpec {
    pub fn option(self) -> char {
        self.option
    }

    pub fn requires_argument(self) -> bool {
        self.requires_argument
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetoptsCaseLabelFact {
    label: char,
    span: Span,
    is_bare_single_letter: bool,
}

impl GetoptsCaseLabelFact {
    pub fn label(self) -> char {
        self.label
    }

    pub fn span(self) -> Span {
        self.span
    }

    pub fn is_bare_single_letter(self) -> bool {
        self.is_bare_single_letter
    }
}

#[derive(Debug, Clone)]
pub struct GetoptsCaseFact {
    case_span: Span,
    declared_options: Box<[GetoptsOptionSpec]>,
    handled_case_labels: Box<[GetoptsCaseLabelFact]>,
    unexpected_case_labels: Box<[GetoptsCaseLabelFact]>,
    invalid_case_pattern_spans: Box<[Span]>,
    has_fallback_pattern: bool,
    has_unknown_coverage: bool,
    missing_options: Box<[GetoptsOptionSpec]>,
}

impl GetoptsCaseFact {
    pub fn case_span(&self) -> Span {
        self.case_span
    }

    pub fn declared_options(&self) -> &[GetoptsOptionSpec] {
        &self.declared_options
    }

    pub fn handled_case_labels(&self) -> &[GetoptsCaseLabelFact] {
        &self.handled_case_labels
    }

    pub fn unexpected_case_labels(&self) -> &[GetoptsCaseLabelFact] {
        &self.unexpected_case_labels
    }

    pub fn invalid_case_pattern_spans(&self) -> &[Span] {
        &self.invalid_case_pattern_spans
    }

    pub fn has_fallback_pattern(&self) -> bool {
        self.has_fallback_pattern
    }

    pub fn has_unknown_coverage(&self) -> bool {
        self.has_unknown_coverage
    }

    pub fn missing_invalid_flag_handler(&self) -> bool {
        !self.has_fallback_pattern
            && !self.has_unknown_coverage
            && !self
                .handled_case_labels
                .iter()
                .any(|label| label.label == '?')
    }

    pub fn missing_options(&self) -> &[GetoptsOptionSpec] {
        &self.missing_options
    }
}


pub(super) fn build_case_item_facts(
    commands: &[CommandFact<'_>],
    arena_file: &ArenaFile,
    source: &str,
) -> Vec<CaseItemFact> {
    let command_ids_by_arena_id = commands
        .iter()
        .filter_map(|fact| Some((fact.arena_command_id()?.index(), fact.id())))
        .collect::<FxHashMap<_, _>>();
    let mut facts = Vec::new();
    collect_arena_case_item_facts(
        arena_file.view().body(),
        &command_ids_by_arena_id,
        source,
        &mut facts,
    );
    facts
}

fn collect_arena_case_item_facts(
    seq: StmtSeqView<'_>,
    command_ids_by_arena_id: &FxHashMap<usize, CommandId>,
    source: &str,
    facts: &mut Vec<CaseItemFact>,
) {
    for stmt in seq.stmts() {
        collect_arena_case_item_facts_from_command(
            stmt.command(),
            command_ids_by_arena_id,
            source,
            facts,
        );
    }
}

fn collect_arena_case_item_facts_from_command(
    command: CommandView<'_>,
    command_ids_by_arena_id: &FxHashMap<usize, CommandId>,
    source: &str,
    facts: &mut Vec<CaseItemFact>,
) {
    if let Some(compound) = command.compound()
        && let CompoundCommandNode::Case { cases, .. } = compound.node()
        && let Some(command_id) = command_ids_by_arena_id.get(&command.id().index()).copied()
    {
        let case_span = trim_trailing_whitespace_span(command.span(), source);
        for item in command.store().case_items(*cases) {
            let patterns = command.store().patterns(item.patterns);
            let mut suspicious_bracket_glob_spans = Vec::new();
            for pattern in patterns {
                collect_arena_pattern_suspicious_bracket_glob_spans(
                    command.store(),
                    pattern,
                    source,
                    &mut suspicious_bracket_glob_spans,
                );
            }
            facts.push(CaseItemFact {
                command_id,
                case_span,
                pattern_spans: patterns
                    .iter()
                    .map(|pattern| pattern.span)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                body_span: command.store().stmt_seq(item.body).span(),
                first_body_stmt_span: command
                    .store()
                    .stmt_seq(item.body)
                    .stmts()
                    .next()
                    .map(|stmt| stmt.span()),
                terminator: item.terminator,
                terminator_span: item.terminator_span,
                suspicious_bracket_glob_spans: suspicious_bracket_glob_spans.into_boxed_slice(),
            });
        }
    }

    for child in command.child_sequences() {
        collect_arena_case_item_facts(child, command_ids_by_arena_id, source, facts);
    }
}

fn collect_arena_pattern_suspicious_bracket_glob_spans(
    store: &AstStore,
    pattern: &PatternNode,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for part in store.pattern_parts(pattern.parts) {
        match &part.kind {
            PatternPartArena::Group { patterns, .. } => {
                for pattern in store.patterns(*patterns) {
                    collect_arena_pattern_suspicious_bracket_glob_spans(
                        store, pattern, source, spans,
                    );
                }
            }
            PatternPartArena::CharClass(_)
                if word_spans::suspicious_bracket_glob_text(part.span.slice(source)) =>
            {
                spans.push(part.span);
            }
            PatternPartArena::Word(_)
            | PatternPartArena::CharClass(_)
            | PatternPartArena::Literal(_)
            | PatternPartArena::AnyString
            | PatternPartArena::AnyChar => {}
        }
    }
}


fn pattern_contains_word_or_group(pattern: &Pattern) -> bool {
    pattern.parts.iter().any(|part| match &part.kind {
        PatternPart::Word(_) => true,
        PatternPart::Group { patterns, .. } => patterns.iter().any(pattern_contains_word_or_group),
        PatternPart::Literal(_)
        | PatternPart::AnyString
        | PatternPart::AnyChar
        | PatternPart::CharClass(_) => false,
    })
}

#[derive(Debug, Clone)]
struct StaticCasePatternMatcher {
    tokens: Vec<CasePatternToken>,
    min_len: usize,
    max_len: Option<usize>,
    literal_prefix: Box<str>,
    literal_suffix: Box<str>,
    literal_symbols: Box<[char]>,
    start_states: Box<[usize]>,
}

#[derive(Debug, Clone)]
struct StaticCasePatternSummary {
    min_len: usize,
    max_len: Option<usize>,
    literal_prefix: Box<str>,
    literal_suffix: Box<str>,
    literal_symbols: Box<[char]>,
    start_states: Box<[usize]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CasePatternToken {
    Literal(char),
    AnyChar,
    AnyString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CasePatternSymbol {
    Literal(char),
    Other,
}

type CasePatternStates = SmallVec<[usize; 8]>;

#[derive(Debug, Clone)]
struct ReachableCasePattern {
    span: Span,
    matcher: StaticCasePatternMatcher,
}

impl StaticCasePatternMatcher {
    fn from_arena_pattern(
        store: &AstStore,
        pattern: &PatternNode,
        source: &str,
    ) -> Option<Self> {
        ensure_arena_case_pattern_is_statically_analyzable(store, pattern, source)?;

        let mut tokens = Vec::new();
        collect_static_case_pattern_tokens(pattern.span.slice(source), &mut tokens)?;
        let StaticCasePatternSummary {
            min_len,
            max_len,
            literal_prefix,
            literal_suffix,
            literal_symbols,
            start_states,
        } = summarize_static_case_pattern_tokens(&tokens);
        Some(Self {
            tokens,
            min_len,
            max_len,
            literal_prefix,
            literal_suffix,
            literal_symbols,
            start_states,
        })
    }

    fn from_arena_case_subject(word: WordView<'_>, source: &str) -> Option<Self> {
        let mut tokens = Vec::new();
        let mut saw_dynamic = false;
        collect_arena_static_case_subject_tokens(
            word.parts(),
            word.store(),
            source,
            &mut tokens,
            &mut saw_dynamic,
        )?;
        if !saw_dynamic {
            return None;
        }

        let StaticCasePatternSummary {
            min_len,
            max_len,
            literal_prefix,
            literal_suffix,
            literal_symbols,
            start_states,
        } = summarize_static_case_pattern_tokens(&tokens);
        Some(Self {
            tokens,
            min_len,
            max_len,
            literal_prefix,
            literal_suffix,
            literal_symbols,
            start_states,
        })
    }

    fn subsumes(&self, other: &Self) -> bool {
        if !self.could_subsume(other) {
            return false;
        }

        let symbols = merged_case_pattern_symbols(
            self.literal_symbols.as_ref(),
            other.literal_symbols.as_ref(),
        );

        let start = (
            case_pattern_states_from_slice(self.start_states.as_ref()),
            case_pattern_states_from_slice(other.start_states.as_ref()),
        );
        let mut seen = FxHashSet::default();
        let mut worklist = vec![start.clone()];
        seen.insert(start);

        while let Some((left, right)) = worklist.pop() {
            if other.is_accepting(&right) && !self.is_accepting(&left) {
                return false;
            }

            for symbol in symbols.iter().copied() {
                let next_right = other.advance(&right, symbol);
                if next_right.is_empty() {
                    continue;
                }

                let next_left = self.advance(&left, symbol);
                if seen.insert((next_left.clone(), next_right.clone())) {
                    worklist.push((next_left, next_right));
                }
            }
        }

        true
    }

    fn intersects(&self, other: &Self) -> bool {
        let symbols = merged_case_pattern_symbols(
            self.literal_symbols.as_ref(),
            other.literal_symbols.as_ref(),
        );

        let start = (
            case_pattern_states_from_slice(self.start_states.as_ref()),
            case_pattern_states_from_slice(other.start_states.as_ref()),
        );
        let mut seen = FxHashSet::default();
        let mut worklist = vec![start.clone()];
        seen.insert(start);

        while let Some((left, right)) = worklist.pop() {
            if self.is_accepting(&left) && other.is_accepting(&right) {
                return true;
            }

            for symbol in symbols.iter().copied() {
                let next_left = self.advance(&left, symbol);
                if next_left.is_empty() {
                    continue;
                }

                let next_right = other.advance(&right, symbol);
                if next_right.is_empty() {
                    continue;
                }

                if seen.insert((next_left.clone(), next_right.clone())) {
                    worklist.push((next_left, next_right));
                }
            }
        }

        false
    }

    fn could_subsume(&self, other: &Self) -> bool {
        if self.min_len > other.min_len {
            return false;
        }
        match (self.max_len, other.max_len) {
            (Some(_), None) => return false,
            (Some(self_max), Some(other_max)) if self_max < other_max => return false,
            (Some(_), Some(_)) | (None, Some(_)) | (None, None) => {}
        }
        if !self.literal_prefix.is_empty()
            && !other
                .literal_prefix
                .starts_with(self.literal_prefix.as_ref())
        {
            return false;
        }
        if !self.literal_suffix.is_empty()
            && !other.literal_suffix.ends_with(self.literal_suffix.as_ref())
        {
            return false;
        }

        true
    }

    fn advance(&self, states: &[usize], symbol: CasePatternSymbol) -> CasePatternStates {
        let mut next = CasePatternStates::new();

        for &state in states {
            let Some(token) = self.tokens.get(state) else {
                continue;
            };

            match token {
                CasePatternToken::Literal(expected) if matches!(symbol, CasePatternSymbol::Literal(actual) if actual == *expected) =>
                {
                    next.push(state + 1);
                }
                CasePatternToken::AnyChar => next.push(state + 1),
                CasePatternToken::AnyString => next.push(state),
                CasePatternToken::Literal(_) => {}
            }
        }

        if next.is_empty() {
            return CasePatternStates::new();
        }

        self.epsilon_closure(next)
    }

    fn epsilon_closure(&self, seeds: impl IntoIterator<Item = usize>) -> CasePatternStates {
        case_pattern_epsilon_closure(&self.tokens, seeds)
    }

    fn is_accepting(&self, states: &[usize]) -> bool {
        states.contains(&self.tokens.len())
    }
}

fn summarize_static_case_pattern_tokens(tokens: &[CasePatternToken]) -> StaticCasePatternSummary {
    let mut min_len = 0usize;
    let mut max_len = Some(0usize);
    let mut literal_prefix = String::new();
    let mut saw_wildcard = false;
    let mut literal_suffix_reversed = String::new();
    let mut saw_suffix_wildcard = false;
    let mut literal_symbols = Vec::new();

    for token in tokens {
        match token {
            CasePatternToken::Literal(ch) => {
                min_len += 1;
                if let Some(max_len) = &mut max_len {
                    *max_len += 1;
                }
                if !saw_wildcard {
                    literal_prefix.push(*ch);
                }
                literal_symbols.push(*ch);
            }
            CasePatternToken::AnyChar => {
                min_len += 1;
                if let Some(max_len) = &mut max_len {
                    *max_len += 1;
                }
                saw_wildcard = true;
            }
            CasePatternToken::AnyString => {
                max_len = None;
                saw_wildcard = true;
            }
        }
    }

    for token in tokens.iter().rev() {
        match token {
            CasePatternToken::Literal(ch) if !saw_suffix_wildcard => {
                literal_suffix_reversed.push(*ch);
            }
            CasePatternToken::Literal(_)
            | CasePatternToken::AnyChar
            | CasePatternToken::AnyString => {
                saw_suffix_wildcard = true;
            }
        }
    }

    literal_symbols.sort_unstable();
    literal_symbols.dedup();

    StaticCasePatternSummary {
        min_len,
        max_len,
        literal_prefix: literal_prefix.into_boxed_str(),
        literal_suffix: literal_suffix_reversed
            .chars()
            .rev()
            .collect::<String>()
            .into_boxed_str(),
        literal_symbols: literal_symbols.into_boxed_slice(),
        start_states: case_pattern_epsilon_closure(tokens, [0]).into_boxed_slice(),
    }
}

fn case_pattern_epsilon_closure(
    tokens: &[CasePatternToken],
    seeds: impl IntoIterator<Item = usize>,
) -> CasePatternStates {
    let mut seen = SmallVec::<[bool; 16]>::new();
    seen.resize(tokens.len() + 1, false);
    let mut states = CasePatternStates::new();
    let mut stack = CasePatternStates::new();

    for state in seeds {
        push_case_pattern_state(&mut seen, &mut states, &mut stack, state);
    }

    while let Some(state) = stack.pop() {
        if matches!(tokens.get(state), Some(CasePatternToken::AnyString)) {
            push_case_pattern_state(&mut seen, &mut states, &mut stack, state + 1);
        }
    }

    states.sort_unstable();
    states
}

fn case_pattern_states_from_slice(states: &[usize]) -> CasePatternStates {
    states.iter().copied().collect()
}

fn push_case_pattern_state(
    seen: &mut [bool],
    states: &mut CasePatternStates,
    stack: &mut CasePatternStates,
    state: usize,
) {
    if let Some(present) = seen.get_mut(state)
        && !*present
    {
        *present = true;
        states.push(state);
        stack.push(state);
    }
}

fn merged_case_pattern_symbols(left: &[char], right: &[char]) -> Vec<CasePatternSymbol> {
    let mut symbols = Vec::with_capacity(left.len() + right.len() + 1);
    let mut left_index = 0usize;
    let mut right_index = 0usize;

    while left_index < left.len() && right_index < right.len() {
        match left[left_index].cmp(&right[right_index]) {
            std::cmp::Ordering::Less => {
                symbols.push(CasePatternSymbol::Literal(left[left_index]));
                left_index += 1;
            }
            std::cmp::Ordering::Greater => {
                symbols.push(CasePatternSymbol::Literal(right[right_index]));
                right_index += 1;
            }
            std::cmp::Ordering::Equal => {
                symbols.push(CasePatternSymbol::Literal(left[left_index]));
                left_index += 1;
                right_index += 1;
            }
        }
    }

    for &symbol in &left[left_index..] {
        symbols.push(CasePatternSymbol::Literal(symbol));
    }
    for &symbol in &right[right_index..] {
        symbols.push(CasePatternSymbol::Literal(symbol));
    }
    symbols.push(CasePatternSymbol::Other);

    symbols
}

fn ensure_case_pattern_is_statically_analyzable(pattern: &Pattern, source: &str) -> Option<()> {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Literal(_) | PatternPart::AnyString | PatternPart::AnyChar => {}
            PatternPart::Word(word) => {
                static_word_text(word, source)?;
            }
            PatternPart::Group { .. } | PatternPart::CharClass(_) => return None,
        }
    }

    Some(())
}

fn ensure_arena_case_pattern_is_statically_analyzable(
    store: &AstStore,
    pattern: &PatternNode,
    source: &str,
) -> Option<()> {
    for part in store.pattern_parts(pattern.parts) {
        match &part.kind {
            PatternPartArena::Literal(_)
            | PatternPartArena::AnyString
            | PatternPartArena::AnyChar => {}
            PatternPartArena::Word(word) => {
                static_word_text_arena(store.word(*word), source)?;
            }
            PatternPartArena::Group { .. } | PatternPartArena::CharClass(_) => return None,
        }
    }

    Some(())
}

fn collect_static_case_pattern_tokens(
    pattern_syntax: &str,
    out: &mut Vec<CasePatternToken>,
) -> Option<()> {
    let mut chars = pattern_syntax.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some('\n') => {}
                Some(escaped) => push_case_pattern_literal_tokens_char(escaped, out),
                None => push_case_pattern_literal_tokens_char('\\', out),
            },
            '\'' => {
                for quoted in chars.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    push_case_pattern_literal_tokens_char(quoted, out);
                }
            }
            '"' => {
                while let Some(quoted) = chars.next() {
                    match quoted {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('\n') => {}
                            Some(escaped @ ('$' | '`' | '"' | '\\')) => {
                                push_case_pattern_literal_tokens_char(escaped, out);
                            }
                            Some(other) => {
                                push_case_pattern_literal_tokens_char('\\', out);
                                push_case_pattern_literal_tokens_char(other, out);
                            }
                            None => push_case_pattern_literal_tokens_char('\\', out),
                        },
                        _ => push_case_pattern_literal_tokens_char(quoted, out),
                    }
                }
            }
            '[' => return None,
            '?' => {
                if chars.peek() == Some(&'(') {
                    return None;
                }
                push_case_pattern_token(out, CasePatternToken::AnyChar);
            }
            '*' => {
                if chars.peek() == Some(&'(') {
                    return None;
                }
                push_case_pattern_token(out, CasePatternToken::AnyString);
            }
            '+' | '@' | '!' if chars.peek() == Some(&'(') => return None,
            '$' | '`' => return None,
            other => push_case_pattern_literal_tokens_char(other, out),
        }
    }
    Some(())
}

fn collect_arena_static_case_subject_tokens(
    parts: &[WordPartArenaNode],
    store: &AstStore,
    source: &str,
    out: &mut Vec<CasePatternToken>,
    saw_dynamic: &mut bool,
) -> Option<()> {
    for part in parts {
        match &part.kind {
            WordPartArena::Literal(text) => {
                for ch in text.as_str(source, part.span).chars() {
                    push_case_pattern_literal_tokens_char(ch, out);
                }
            }
            WordPartArena::SingleQuoted { value, .. } => {
                for ch in value.slice(source).chars() {
                    push_case_pattern_literal_tokens_char(ch, out);
                }
            }
            WordPartArena::DoubleQuoted { parts, .. } => {
                collect_arena_static_case_subject_tokens(
                    store.word_parts(*parts),
                    store,
                    source,
                    out,
                    saw_dynamic,
                )?;
            }
            WordPartArena::Variable(_)
            | WordPartArena::CommandSubstitution { .. }
            | WordPartArena::ArithmeticExpansion { .. }
            | WordPartArena::Parameter(_)
            | WordPartArena::ParameterExpansion { .. }
            | WordPartArena::Length(_)
            | WordPartArena::ArrayAccess(_)
            | WordPartArena::ArrayLength(_)
            | WordPartArena::ArrayIndices(_)
            | WordPartArena::Substring { .. }
            | WordPartArena::ArraySlice { .. }
            | WordPartArena::IndirectExpansion { .. }
            | WordPartArena::PrefixMatch { .. }
            | WordPartArena::ProcessSubstitution { .. }
            | WordPartArena::Transformation { .. } => {
                *saw_dynamic = true;
                push_case_pattern_token(out, CasePatternToken::AnyString);
            }
            WordPartArena::ZshQualifiedGlob(_) => return None,
        }
    }

    Some(())
}

fn push_case_pattern_literal_tokens_char(ch: char, out: &mut Vec<CasePatternToken>) {
    out.push(CasePatternToken::Literal(ch));
}

fn push_case_pattern_token(out: &mut Vec<CasePatternToken>, token: CasePatternToken) {
    if matches!(token, CasePatternToken::AnyString)
        && matches!(out.last(), Some(CasePatternToken::AnyString))
    {
        return;
    }

    out.push(token);
}

fn build_case_pattern_shadow_facts(
    commands: &[CommandFact<'_>],
    arena_file: &ArenaFile,
    source: &str,
) -> Vec<CasePatternShadowFact> {
    let mut shadows = Vec::new();

    for fact in commands {
        let Some(command) = fact
            .arena_command_id()
            .map(|id| arena_file.store.command(id))
        else {
            continue;
        };
        let Some(compound) = command.compound() else {
            continue;
        };
        let CompoundCommandNode::Case { cases, .. } = compound.node() else {
            continue;
        };

        let mut prior_arm_patterns = Vec::<ReachableCasePattern>::new();
        let mut fallthrough_arm_patterns = Vec::<ReachableCasePattern>::new();
        let mut spent_shadowing_patterns = FxHashSet::default();

        for item in command.store().case_items(*cases) {
            let mut same_item_patterns = Vec::<ReachableCasePattern>::new();

            for pattern in command.store().patterns(item.patterns) {
                let Some(matcher) =
                    StaticCasePatternMatcher::from_arena_pattern(command.store(), pattern, source)
                else {
                    continue;
                };

                for previous in prior_arm_patterns
                    .iter()
                    .chain(fallthrough_arm_patterns.iter())
                    .chain(same_item_patterns.iter())
                {
                    if spent_shadowing_patterns.contains(&FactSpan::new(previous.span)) {
                        continue;
                    }

                    if previous.matcher.subsumes(&matcher) {
                        shadows.push(CasePatternShadowFact {
                            shadowing_pattern_span: previous.span,
                            shadowed_pattern_span: pattern.span,
                        });
                        spent_shadowing_patterns.insert(FactSpan::new(previous.span));
                        break;
                    }
                }

                same_item_patterns.push(ReachableCasePattern {
                    span: pattern.span,
                    matcher,
                });
            }

            match item.terminator {
                CaseTerminator::Break => {
                    prior_arm_patterns.append(&mut fallthrough_arm_patterns);
                    prior_arm_patterns.extend(same_item_patterns);
                }
                CaseTerminator::FallThrough => {
                    fallthrough_arm_patterns.extend(same_item_patterns);
                }
                CaseTerminator::Continue | CaseTerminator::ContinueMatching => {
                    fallthrough_arm_patterns.clear();
                }
            }
        }
    }

    shadows
}

fn build_case_pattern_impossible_spans(
    commands: &[CommandFact<'_>],
    arena_file: &ArenaFile,
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();

    for fact in commands {
        let Some(command) = fact
            .arena_command_id()
            .map(|id| arena_file.store.command(id))
        else {
            continue;
        };
        let Some(compound) = command.compound() else {
            continue;
        };
        let CompoundCommandNode::Case { word, cases, .. } = compound.node() else {
            continue;
        };

        let Some(subject_matcher) =
            StaticCasePatternMatcher::from_arena_case_subject(command.store().word(*word), source)
        else {
            continue;
        };

        for item in command.store().case_items(*cases) {
            for pattern in command.store().patterns(item.patterns) {
                let Some(pattern_matcher) =
                    StaticCasePatternMatcher::from_arena_pattern(command.store(), pattern, source)
                else {
                    continue;
                };

                if !subject_matcher.intersects(&pattern_matcher) {
                    spans.push(pattern.span);
                }
            }
        }
    }

    spans
}

#[derive(Debug, Clone)]
struct ParsedGetoptsCommand {
    declared_options: Vec<GetoptsOptionSpec>,
    target_name: Name,
}

#[derive(Debug, Clone)]
struct GetoptsCaseMatch {
    case_span: Span,
    handled_case_labels: Vec<GetoptsCaseLabelFact>,
    invalid_case_pattern_spans: Vec<Span>,
    has_fallback_pattern: bool,
    has_unknown_coverage: bool,
}

fn build_getopts_case_fact_for_while(
    command: &WhileCommand,
    source: &str,
) -> Option<GetoptsCaseFact> {
    let parsed = parse_getopts_command_from_condition(&command.condition, source)?;
    let GetoptsCaseMatch {
        case_span,
        handled_case_labels,
        invalid_case_pattern_spans,
        has_fallback_pattern,
        has_unknown_coverage,
    } = first_getopts_case_match(&command.body, parsed.target_name.as_str(), source)?;

    let handled = handled_case_labels
        .iter()
        .map(|label| label.label)
        .collect::<FxHashSet<_>>();
    let declared = parsed
        .declared_options
        .iter()
        .map(|option| option.option)
        .collect::<FxHashSet<_>>();
    let unexpected_case_labels = handled_case_labels
        .iter()
        .copied()
        .filter(|label| !declared.contains(&label.label()))
        .filter(|label| !matches!(label.label(), '?' | ':'))
        .collect::<Vec<_>>();
    let missing_options = if has_fallback_pattern || has_unknown_coverage {
        Vec::new()
    } else {
        parsed
            .declared_options
            .iter()
            .copied()
            .filter(|option| !handled.contains(&option.option))
            .collect::<Vec<_>>()
    };

    Some(GetoptsCaseFact {
        case_span,
        declared_options: parsed.declared_options.into_boxed_slice(),
        handled_case_labels: handled_case_labels.into_boxed_slice(),
        unexpected_case_labels: unexpected_case_labels.into_boxed_slice(),
        invalid_case_pattern_spans: invalid_case_pattern_spans.into_boxed_slice(),
        has_fallback_pattern,
        has_unknown_coverage,
        missing_options: missing_options.into_boxed_slice(),
    })
}

fn parse_getopts_command_from_condition(
    condition: &StmtSeq,
    source: &str,
) -> Option<ParsedGetoptsCommand> {
    let stmt = condition.last()?;
    let normalized = command::normalize_command(&stmt.command, source);
    if !normalized.effective_name_is("getopts") {
        return None;
    }

    let args = normalized.body_args();
    let option_string = static_word_text(args.first()?, source)?;
    let target_text = static_word_text(args.get(1)?, source)?;
    if !is_shell_variable_name(&target_text) {
        return None;
    }

    let declared_options = parse_getopts_option_specs(&option_string);
    Some(ParsedGetoptsCommand {
        declared_options,
        target_name: Name::from(target_text),
    })
}

fn parse_getopts_option_specs(option_string: &str) -> Vec<GetoptsOptionSpec> {
    let mut specs = Vec::new();
    let mut seen = FxHashSet::default();
    let mut chars = option_string.chars().peekable();

    if chars.peek() == Some(&':') {
        chars.next();
    }

    while let Some(option) = chars.next() {
        if option == ':' {
            continue;
        }

        let requires_argument = chars.peek() == Some(&':');
        if requires_argument {
            chars.next();
        }

        if seen.insert(option) {
            specs.push(GetoptsOptionSpec {
                option,
                requires_argument,
            });
        }
    }

    specs
}

fn first_getopts_case_match(
    body: &StmtSeq,
    target_name: &str,
    source: &str,
) -> Option<GetoptsCaseMatch> {
    first_getopts_case_match_in_commands(body, target_name, source)
}

fn first_getopts_case_match_in_commands(
    commands: &StmtSeq,
    target_name: &str,
    source: &str,
) -> Option<GetoptsCaseMatch> {
    commands
        .iter()
        .find_map(|stmt| first_getopts_case_match_in_command(&stmt.command, target_name, source))
}

fn first_getopts_case_match_in_command(
    command: &Command,
    target_name: &str,
    source: &str,
) -> Option<GetoptsCaseMatch> {
    match command {
        Command::Binary(command) => first_getopts_case_match_in_command(
            &command.left.command,
            target_name,
            source,
        )
        .or_else(|| {
            matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll).then(|| {
                first_getopts_case_match_in_command(&command.right.command, target_name, source)
            })?
        }),
        Command::Compound(CompoundCommand::Case(command))
            if case_subject_variable_name(&command.word) == Some(target_name) =>
        {
            Some(build_getopts_case_match(command, source))
        }
        Command::Compound(CompoundCommand::BraceGroup(commands)) => {
            first_getopts_case_match_in_commands(commands, target_name, source)
        }
        // Helper definitions are not part of the executable getopts dispatch path.
        Command::Function(_) | Command::AnonymousFunction(_) => None,
        Command::Compound(_) | Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => None,
    }
}

fn build_getopts_case_match(command: &CaseCommand, source: &str) -> GetoptsCaseMatch {
    let mut has_fallback_pattern = false;
    let mut has_unknown_coverage = false;
    let mut invalid_case_pattern_spans = Vec::new();
    let labels = command
        .cases
        .iter()
        .flat_map(|item| item.patterns.iter())
        .filter_map(
            |pattern| match classify_getopts_case_pattern(pattern, source) {
                GetoptsCasePatternKind::Fallback => {
                    has_fallback_pattern = true;
                    None
                }
                GetoptsCasePatternKind::SingleLabel(label) => Some(label),
                GetoptsCasePatternKind::InvalidStaticPattern(span) => {
                    invalid_case_pattern_spans.push(span);
                    None
                }
                GetoptsCasePatternKind::UnknownCoverage => {
                    has_unknown_coverage = true;
                    None
                }
            },
        )
        .collect::<Vec<_>>();
    GetoptsCaseMatch {
        case_span: trim_trailing_case_span(command.span, source),
        handled_case_labels: labels,
        invalid_case_pattern_spans,
        has_fallback_pattern,
        has_unknown_coverage,
    }
}

fn trim_trailing_case_span(span: Span, source: &str) -> Span {
    let text = span.slice(source);
    let mut line_start = 0;
    let mut last_code_end = 0;

    for line in text.split_inclusive('\n') {
        let line_end = line_start + line.len();
        let line_without_newline = line.trim_end_matches(['\r', '\n']);
        let line_without_comment =
            trim_case_line_comment(line_without_newline).trim_end_matches([' ', '\t']);

        if !line_without_comment
            .trim_start_matches([' ', '\t'])
            .is_empty()
        {
            last_code_end = line_start + line_without_comment.len();
        }

        line_start = line_end;
    }

    if last_code_end == 0 {
        return span;
    }

    Span::from_positions(span.start, span.start.advanced_by(&text[..last_code_end]))
}

fn trim_case_line_comment(line: &str) -> &str {
    for (index, ch) in line.char_indices() {
        if ch == '#'
            && line[..index]
                .chars()
                .next_back()
                .is_none_or(char::is_whitespace)
        {
            return &line[..index];
        }
    }

    line
}

enum GetoptsCasePatternKind {
    Fallback,
    SingleLabel(GetoptsCaseLabelFact),
    InvalidStaticPattern(Span),
    UnknownCoverage,
}

fn classify_getopts_case_pattern(pattern: &Pattern, source: &str) -> GetoptsCasePatternKind {
    if getopts_case_pattern_is_fallback(pattern, source) {
        return GetoptsCasePatternKind::Fallback;
    }

    let Some(text) = static_case_pattern_text(pattern, source) else {
        return GetoptsCasePatternKind::UnknownCoverage;
    };
    let mut chars = text.chars();
    let Some(label) = chars.next() else {
        return GetoptsCasePatternKind::UnknownCoverage;
    };
    if chars.next().is_some() {
        return GetoptsCasePatternKind::InvalidStaticPattern(pattern.span);
    }

    let is_bare_single_letter = label.is_ascii_alphabetic() && pattern.span.slice(source) == text;
    GetoptsCasePatternKind::SingleLabel(GetoptsCaseLabelFact {
        label,
        span: pattern.span,
        is_bare_single_letter,
    })
}

fn getopts_case_pattern_is_fallback(pattern: &Pattern, source: &str) -> bool {
    let mut tokens = Vec::new();
    if collect_static_case_pattern_tokens(pattern.span.slice(source), &mut tokens).is_none() {
        return false;
    }

    matches!(
        tokens.as_slice(),
        [CasePatternToken::AnyString] | [CasePatternToken::AnyChar]
    )
}

fn static_case_pattern_text(pattern: &Pattern, source: &str) -> Option<String> {
    ensure_case_pattern_is_statically_analyzable(pattern, source)?;

    let mut tokens = Vec::new();
    collect_static_case_pattern_tokens(pattern.span.slice(source), &mut tokens)?;
    tokens
        .into_iter()
        .map(|token| match token {
            CasePatternToken::Literal(ch) => Some(ch),
            CasePatternToken::AnyChar | CasePatternToken::AnyString => None,
        })
        .collect()
}

fn case_subject_variable_name(word: &Word) -> Option<&str> {
    standalone_variable_name_from_word_parts(&word.parts)
}
