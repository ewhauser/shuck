#[derive(Debug, Clone, Copy)]
pub struct CaseItemFact<'a> {
    item: &'a CaseItem,
    command_id: CommandId,
    case_span: Span,
}

impl<'a> CaseItemFact<'a> {
    pub fn item(&self) -> &'a CaseItem {
        self.item
    }

    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn terminator(&self) -> CaseTerminator {
        self.item.terminator
    }

    pub fn terminator_span(&self) -> Option<Span> {
        self.item.terminator_span
    }

    pub fn case_span(&self) -> Span {
        self.case_span
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


pub(super) fn build_case_item_facts<'a>(
    commands: &[CommandFact<'a>],
    source: &str,
) -> Vec<CaseItemFact<'a>> {
    commands
        .iter()
        .filter_map(|fact| {
            let Command::Compound(CompoundCommand::Case(command)) = fact.command() else {
                return None;
            };

            let case_span = fact.span_in_source(source);
            Some(command.cases.iter().map(move |item| CaseItemFact {
                item,
                command_id: fact.id(),
                case_span,
            }))
        })
        .flatten()
        .collect()
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
    bit_nfa: Option<StaticCasePatternBitNfa>,
}

#[derive(Debug, Clone)]
struct StaticCasePatternBitNfa {
    accept: u128,
    start: u128,
    any_char: u128,
    any_string: u128,
    literal_masks: Box<[(char, u128)]>,
}

impl StaticCasePatternBitNfa {
    fn new(tokens: &[CasePatternToken]) -> Option<Self> {
        if tokens.len() >= 128 {
            return None;
        }

        let mut any_char = 0u128;
        let mut any_string = 0u128;
        let mut literal_masks = Vec::<(char, u128)>::new();

        for (i, token) in tokens.iter().copied().enumerate() {
            let bit = 1u128 << i;
            match token {
                CasePatternToken::Literal(c) => literal_masks.push((c, bit)),
                CasePatternToken::AnyChar => any_char |= bit,
                CasePatternToken::AnyString => any_string |= bit,
            }
        }

        literal_masks.sort_by_key(|(c, _)| *c);
        let mut merged: Vec<(char, u128)> = Vec::with_capacity(literal_masks.len());
        for (c, mask) in literal_masks {
            match merged.last_mut() {
                Some((last_c, last_mask)) if *last_c == c => *last_mask |= mask,
                _ => merged.push((c, mask)),
            }
        }

        let mut nfa = Self {
            accept: 1u128 << tokens.len(),
            start: 1,
            any_char,
            any_string,
            literal_masks: merged.into_boxed_slice(),
        };
        nfa.start = nfa.epsilon_closure(nfa.start);
        Some(nfa)
    }

    #[inline]
    fn is_accepting(&self, states: u128) -> bool {
        states & self.accept != 0
    }

    #[inline]
    fn literal_mask(&self, c: char) -> u128 {
        match self
            .literal_masks
            .binary_search_by_key(&c, |(literal, _)| *literal)
        {
            Ok(index) => self.literal_masks[index].1,
            Err(_) => 0,
        }
    }

    #[inline]
    fn epsilon_closure(&self, mut states: u128) -> u128 {
        loop {
            let next = states | ((states & self.any_string) << 1);
            if next == states {
                return states;
            }
            states = next;
        }
    }

    #[inline]
    fn advance(&self, states: u128, symbol: CasePatternSymbol) -> u128 {
        if states == 0 {
            return 0;
        }
        let literal = match symbol {
            CasePatternSymbol::Literal(c) => self.literal_mask(c),
            CasePatternSymbol::Other => 0,
        };
        let shifted = (states & (literal | self.any_char)) << 1;
        let stayed = states & self.any_string;
        self.epsilon_closure(shifted | stayed)
    }
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
    fn from_pattern(pattern: &Pattern, source: &str) -> Option<Self> {
        ensure_case_pattern_is_statically_analyzable(pattern, source)?;

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
        let bit_nfa = StaticCasePatternBitNfa::new(&tokens);
        Some(Self {
            tokens,
            min_len,
            max_len,
            literal_prefix,
            literal_suffix,
            literal_symbols,
            start_states,
            bit_nfa,
        })
    }

    fn from_case_subject(word: &Word, source: &str) -> Option<Self> {
        let mut tokens = Vec::new();
        let mut saw_dynamic = false;
        collect_static_case_subject_tokens(&word.parts, source, &mut tokens, &mut saw_dynamic)?;
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
        let bit_nfa = StaticCasePatternBitNfa::new(&tokens);
        Some(Self {
            tokens,
            min_len,
            max_len,
            literal_prefix,
            literal_suffix,
            literal_symbols,
            start_states,
            bit_nfa,
        })
    }

    fn subsumes(&self, other: &Self) -> bool {
        if !self.could_subsume(other) {
            return false;
        }

        if let Some(result) = subsumes_fixed_length_fast_path(self, other) {
            return result;
        }

        if both_have_simple_glob_form(self, other) {
            return true;
        }

        if let (Some(left_nfa), Some(right_nfa)) = (&self.bit_nfa, &other.bit_nfa) {
            return self.subsumes_bitset(other, left_nfa, right_nfa);
        }

        self.subsumes_slow(other)
    }

    fn subsumes_slow(&self, other: &Self) -> bool {
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

    fn subsumes_bitset(
        &self,
        other: &Self,
        left_nfa: &StaticCasePatternBitNfa,
        right_nfa: &StaticCasePatternBitNfa,
    ) -> bool {
        let symbols = merged_case_pattern_symbols(
            self.literal_symbols.as_ref(),
            other.literal_symbols.as_ref(),
        );

        let start = (left_nfa.start, right_nfa.start);
        let mut seen = FxHashSet::default();
        let mut worklist = Vec::with_capacity(32);
        seen.insert(start);
        worklist.push(start);

        while let Some((left, right)) = worklist.pop() {
            if right_nfa.is_accepting(right) && !left_nfa.is_accepting(left) {
                return false;
            }

            for symbol in symbols.iter().copied() {
                let next_right = right_nfa.advance(right, symbol);
                if next_right == 0 {
                    continue;
                }
                let next_left = left_nfa.advance(left, symbol);
                let next = (next_left, next_right);
                if seen.insert(next) {
                    worklist.push(next);
                }
            }
        }

        true
    }

    fn intersects_bitset(
        &self,
        other: &Self,
        left_nfa: &StaticCasePatternBitNfa,
        right_nfa: &StaticCasePatternBitNfa,
    ) -> bool {
        let symbols = merged_case_pattern_symbols(
            self.literal_symbols.as_ref(),
            other.literal_symbols.as_ref(),
        );

        let start = (left_nfa.start, right_nfa.start);
        let mut seen = FxHashSet::default();
        let mut worklist = Vec::with_capacity(32);
        seen.insert(start);
        worklist.push(start);

        while let Some((left, right)) = worklist.pop() {
            if left_nfa.is_accepting(left) && right_nfa.is_accepting(right) {
                return true;
            }

            for symbol in symbols.iter().copied() {
                let next_left = left_nfa.advance(left, symbol);
                if next_left == 0 {
                    continue;
                }
                let next_right = right_nfa.advance(right, symbol);
                if next_right == 0 {
                    continue;
                }
                let next = (next_left, next_right);
                if seen.insert(next) {
                    worklist.push(next);
                }
            }
        }

        false
    }

    fn intersects(&self, other: &Self) -> bool {
        if !self.could_intersect(other) {
            return false;
        }

        if let Some(result) = intersects_fixed_length_fast_path(self, other) {
            return result;
        }

        if let Some(result) = intersects_simple_glob_fast_path(self, other) {
            return result;
        }

        if let (Some(left_nfa), Some(right_nfa)) = (&self.bit_nfa, &other.bit_nfa) {
            return self.intersects_bitset(other, left_nfa, right_nfa);
        }

        self.intersects_slow(other)
    }

    fn intersects_slow(&self, other: &Self) -> bool {
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

    fn could_intersect(&self, other: &Self) -> bool {
        if matches!(self.max_len, Some(max) if max < other.min_len) {
            return false;
        }
        if matches!(other.max_len, Some(max) if max < self.min_len) {
            return false;
        }
        if !literal_prefixes_compatible(
            self.literal_prefix.as_ref(),
            other.literal_prefix.as_ref(),
        ) {
            return false;
        }
        if !literal_suffixes_compatible(
            self.literal_suffix.as_ref(),
            other.literal_suffix.as_ref(),
        ) {
            return false;
        }
        true
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

#[cfg(feature = "benchmarking")]
pub mod benchmark {
    use super::{
        StaticCasePatternBitNfa, StaticCasePatternMatcher, StaticCasePatternSummary,
        collect_static_case_pattern_tokens, summarize_static_case_pattern_tokens,
    };

    #[doc(hidden)]
    pub struct CasePatternMatcher(StaticCasePatternMatcher);

    impl CasePatternMatcher {
        pub fn from_glob(glob: &str) -> Option<Self> {
            let mut tokens = Vec::new();
            collect_static_case_pattern_tokens(glob, &mut tokens)?;
            let StaticCasePatternSummary {
                min_len,
                max_len,
                literal_prefix,
                literal_suffix,
                literal_symbols,
                start_states,
            } = summarize_static_case_pattern_tokens(&tokens);
            let bit_nfa = StaticCasePatternBitNfa::new(&tokens);
            Some(Self(StaticCasePatternMatcher {
                tokens,
                min_len,
                max_len,
                literal_prefix,
                literal_suffix,
                literal_symbols,
                start_states,
                bit_nfa,
            }))
        }

        pub fn subsumes(&self, other: &Self) -> bool {
            self.0.subsumes(&other.0)
        }

        pub fn intersects(&self, other: &Self) -> bool {
            self.0.intersects(&other.0)
        }
    }
}

fn subsumes_fixed_length_fast_path(
    left: &StaticCasePatternMatcher,
    right: &StaticCasePatternMatcher,
) -> Option<bool> {
    if left.max_len.is_none() || right.max_len.is_none() {
        return None;
    }
    if left.tokens.len() != right.tokens.len() {
        return Some(false);
    }
    let result = left
        .tokens
        .iter()
        .zip(right.tokens.iter())
        .all(|(l, r)| match (l, r) {
            (CasePatternToken::Literal(a), CasePatternToken::Literal(b)) => a == b,
            (CasePatternToken::AnyChar, _) => true,
            (CasePatternToken::Literal(_), CasePatternToken::AnyChar) => false,
            (CasePatternToken::AnyString, _) | (_, CasePatternToken::AnyString) => {
                unreachable!("AnyString tokens excluded by max_len check")
            }
        });
    Some(result)
}

fn glob_simple_form(matcher: &StaticCasePatternMatcher) -> Option<bool> {
    let mut star_count = 0u32;
    for token in &matcher.tokens {
        match token {
            CasePatternToken::Literal(_) => {}
            CasePatternToken::AnyChar => return None,
            CasePatternToken::AnyString => {
                star_count += 1;
                if star_count > 1 {
                    return None;
                }
            }
        }
    }
    Some(star_count == 1)
}

fn both_have_simple_glob_form(
    left: &StaticCasePatternMatcher,
    right: &StaticCasePatternMatcher,
) -> bool {
    glob_simple_form(left).is_some() && glob_simple_form(right).is_some()
}

fn intersects_simple_glob_fast_path(
    left: &StaticCasePatternMatcher,
    right: &StaticCasePatternMatcher,
) -> Option<bool> {
    let lh = glob_simple_form(left)?;
    let rh = glob_simple_form(right)?;
    let lp: &str = left.literal_prefix.as_ref();
    let ls: &str = left.literal_suffix.as_ref();
    let rp: &str = right.literal_prefix.as_ref();
    let rs: &str = right.literal_suffix.as_ref();
    Some(match (lh, rh) {
        (false, false) => lp == rp,
        (false, true) => lp.len() >= rp.len() + rs.len() && lp.starts_with(rp) && lp.ends_with(rs),
        (true, false) => rp.len() >= lp.len() + ls.len() && rp.starts_with(lp) && rp.ends_with(ls),
        (true, true) => {
            (lp.starts_with(rp) || rp.starts_with(lp)) && (ls.ends_with(rs) || rs.ends_with(ls))
        }
    })
}

fn intersects_fixed_length_fast_path(
    left: &StaticCasePatternMatcher,
    right: &StaticCasePatternMatcher,
) -> Option<bool> {
    if left.max_len.is_none() || right.max_len.is_none() {
        return None;
    }
    if left.tokens.len() != right.tokens.len() {
        return Some(false);
    }
    let result = left
        .tokens
        .iter()
        .zip(right.tokens.iter())
        .all(|(l, r)| match (l, r) {
            (CasePatternToken::Literal(a), CasePatternToken::Literal(b)) => a == b,
            (CasePatternToken::AnyChar, _) | (_, CasePatternToken::AnyChar) => true,
            (CasePatternToken::AnyString, _) | (_, CasePatternToken::AnyString) => {
                unreachable!("AnyString tokens excluded by max_len check")
            }
        });
    Some(result)
}

#[inline]
fn literal_prefixes_compatible(left: &str, right: &str) -> bool {
    left.is_empty() || right.is_empty() || left.starts_with(right) || right.starts_with(left)
}

#[inline]
fn literal_suffixes_compatible(left: &str, right: &str) -> bool {
    left.is_empty() || right.is_empty() || left.ends_with(right) || right.ends_with(left)
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

fn collect_static_case_subject_tokens(
    parts: &[WordPartNode],
    source: &str,
    out: &mut Vec<CasePatternToken>,
    saw_dynamic: &mut bool,
) -> Option<()> {
    for part in parts {
        match &part.kind {
            WordPart::Literal(text) => {
                for ch in text.as_str(source, part.span).chars() {
                    push_case_pattern_literal_tokens_char(ch, out);
                }
            }
            WordPart::SingleQuoted { value, .. } => {
                for ch in value.slice(source).chars() {
                    push_case_pattern_literal_tokens_char(ch, out);
                }
            }
            WordPart::DoubleQuoted { parts, .. } => {
                collect_static_case_subject_tokens(parts, source, out, saw_dynamic)?;
            }
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
                *saw_dynamic = true;
                push_case_pattern_token(out, CasePatternToken::AnyString);
            }
            WordPart::ZshQualifiedGlob(_) => return None,
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
    source: &str,
) -> Vec<CasePatternShadowFact> {
    let mut shadows = Vec::new();

    for fact in commands {
        let Command::Compound(CompoundCommand::Case(command)) = fact.command() else {
            continue;
        };

        let mut prior_arm_patterns = Vec::<ReachableCasePattern>::new();
        let mut fallthrough_arm_patterns = Vec::<ReachableCasePattern>::new();
        let mut spent_shadowing_patterns = FxHashSet::default();

        for item in &command.cases {
            let mut same_item_patterns = Vec::<ReachableCasePattern>::new();

            for pattern in &item.patterns {
                let Some(matcher) = StaticCasePatternMatcher::from_pattern(pattern, source) else {
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

fn build_case_pattern_impossible_spans(commands: &[CommandFact<'_>], source: &str) -> Vec<Span> {
    let mut spans = Vec::new();

    for fact in commands {
        let Command::Compound(CompoundCommand::Case(command)) = fact.command() else {
            continue;
        };

        let Some(subject_matcher) =
            StaticCasePatternMatcher::from_case_subject(&command.word, source)
        else {
            continue;
        };

        for item in &command.cases {
            for pattern in &item.patterns {
                let Some(pattern_matcher) = StaticCasePatternMatcher::from_pattern(pattern, source)
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
