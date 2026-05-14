use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectTargetKind {
    File,
    DescriptorDup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum RedirectDevNullStatus {
    DefinitelyDevNull,
    DefinitelyNotDevNull,
    MaybeDevNull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedirectTargetAnalysis {
    pub kind: RedirectTargetKind,
    pub dev_null_status: Option<RedirectDevNullStatus>,
    pub numeric_descriptor_target: Option<i32>,
    pub expansion: ExpansionAnalysis,
    pub runtime_literal: RuntimeLiteralAnalysis,
}

impl RedirectTargetAnalysis {
    pub fn is_descriptor_dup(self) -> bool {
        matches!(self.kind, RedirectTargetKind::DescriptorDup)
    }

    pub fn is_file_target(self) -> bool {
        matches!(self.kind, RedirectTargetKind::File)
    }

    pub fn is_definitely_dev_null(self) -> bool {
        matches!(
            self.dev_null_status,
            Some(RedirectDevNullStatus::DefinitelyDevNull)
        )
    }

    pub fn is_definitely_not_dev_null(self) -> bool {
        matches!(
            self.dev_null_status,
            Some(RedirectDevNullStatus::DefinitelyNotDevNull)
        )
    }

    pub fn is_runtime_sensitive(self) -> bool {
        self.expansion.literalness == WordLiteralness::Expanded
            || self.runtime_literal.is_runtime_sensitive()
            || matches!(
                self.dev_null_status,
                Some(RedirectDevNullStatus::MaybeDevNull)
            )
    }

    pub fn can_expand_to_multiple_fields(self) -> bool {
        self.expansion.can_expand_to_multiple_fields
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ComparablePathKey {
    Literal(Box<str>),
    Parameter(Box<str>),
    Template(Box<[ComparablePathPart]>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ComparablePathMatchKey {
    key: ComparablePathKey,
    quote: WordQuote,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ComparablePathPart {
    Literal(Box<str>),
    Parameter(Box<str>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ComparablePath {
    span: Span,
    key: ComparablePathKey,
    quote: WordQuote,
}

impl ComparablePath {
    pub(crate) fn span(&self) -> Span {
        self.span
    }

    pub(crate) fn key(&self) -> &ComparablePathKey {
        &self.key
    }

    pub(crate) fn match_key(&self) -> ComparablePathMatchKey {
        ComparablePathMatchKey {
            key: self.key.clone(),
            quote: self.quote,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ComparableNameKey(pub(crate) Box<str>);

impl ComparableNameKey {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ComparableNameUseKind {
    Literal,
    Parameter,
    Derived,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ComparableNameUse {
    pub(crate) span: Span,
    pub(crate) key: ComparableNameKey,
    pub(crate) kind: ComparableNameUseKind,
}

impl ComparableNameUse {
    pub(crate) fn span(&self) -> Span {
        self.span
    }

    pub(crate) fn key(&self) -> &ComparableNameKey {
        &self.key
    }

    pub(crate) fn kind(&self) -> ComparableNameUseKind {
        self.kind
    }

    pub(crate) fn mark_derived(&mut self) {
        self.kind = ComparableNameUseKind::Derived;
    }
}

pub(crate) fn comparable_path(
    word: &Word,
    source: &str,
    context: ExpansionContext,
    behavior: Option<&ShellBehaviorAt<'_>>,
) -> Option<ComparablePath> {
    let analysis = analyze_word(word, source, behavior);
    if analysis.has_command_substitution()
        || analysis.hazards.command_or_process_substitution
        || analysis.has_array_expansion()
    {
        return None;
    }

    let runtime_literal = analyze_literal_runtime(word, source, context, behavior);
    if runtime_literal.hazards.pathname_matching
        || runtime_literal.hazards.brace_fanout
        || runtime_literal.hazards.command_or_process_substitution
        || runtime_literal.hazards.arithmetic_expansion
        || runtime_literal.hazards.runtime_pattern
    {
        return None;
    }

    let key = comparable_path_key_from_parts(&word.parts, source)?;
    if comparable_path_key_is_special_device(&key) {
        return None;
    }

    Some(ComparablePath {
        span: word.span,
        key,
        quote: analysis.quote,
    })
}

pub(crate) fn comparable_name_uses(
    word: &Word,
    semantic: Option<&LinterSemanticArtifacts<'_>>,
    source: &str,
) -> Box<[ComparableNameUse]> {
    let mut uses = Vec::new();
    if let Some(name_use) = standalone_comparable_name_use(word, source) {
        uses.push(name_use);
    }
    let allow_quoted_derived_words =
        analyze_word(word, source, None).quote == WordQuote::FullyQuoted;
    collect_command_substitution_comparable_name_uses_in_parts(
        &word.parts,
        semantic,
        source,
        allow_quoted_derived_words,
        &mut uses,
    );
    dedup_comparable_name_uses(&mut uses);
    uses.into_boxed_slice()
}

pub(crate) fn comparable_read_target_name_uses(
    word: &Word,
    semantic: &LinterSemanticArtifacts<'_>,
    source: &str,
) -> Box<[ComparableNameUse]> {
    comparable_name_uses_with_quoted_literals(word, Some(semantic), source)
}

pub(crate) fn comparable_heredoc_name_uses(
    heredoc: &shuck_ast::HeredocBody,
    semantic: Option<&LinterSemanticArtifacts<'_>>,
    locator: Locator<'_>,
) -> Box<[ComparableNameUse]> {
    let source = locator.source();
    let mut uses = Vec::new();
    for part in &heredoc.parts {
        match &part.kind {
            shuck_ast::HeredocBodyPart::Variable(name) => {
                if comparable_name_text(name.as_str()) {
                    uses.push(ComparableNameUse {
                        span: heredoc_variable_name_span(part.span, locator),
                        key: ComparableNameKey(name.as_str().into()),
                        kind: ComparableNameUseKind::Parameter,
                    });
                }
            }
            shuck_ast::HeredocBodyPart::Parameter(parameter) => {
                if let Some(name) = comparable_name_from_parameter(parameter) {
                    uses.push(ComparableNameUse {
                        span: part.span,
                        key: ComparableNameKey(name.into()),
                        kind: ComparableNameUseKind::Parameter,
                    });
                }
            }
            shuck_ast::HeredocBodyPart::CommandSubstitution { body, .. } => {
                collect_command_substitution_comparable_name_uses(
                    body, semantic, source, true, &mut uses,
                );
            }
            shuck_ast::HeredocBodyPart::ArithmeticExpansion {
                expression_word_ast,
                ..
            } => {
                if let Some(name_use) = standalone_comparable_name_use(expression_word_ast, source)
                {
                    uses.push(name_use);
                }
            }
            shuck_ast::HeredocBodyPart::Literal(_) => {}
        }
    }
    dedup_comparable_name_uses(&mut uses);
    uses.into_boxed_slice()
}

pub(crate) fn collect_command_substitution_comparable_name_uses_in_parts(
    parts: &[WordPartNode],
    semantic: Option<&LinterSemanticArtifacts<'_>>,
    source: &str,
    allow_quoted_derived_words: bool,
    uses: &mut Vec<ComparableNameUse>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_command_substitution_comparable_name_uses_in_parts(
                    parts,
                    semantic,
                    source,
                    allow_quoted_derived_words,
                    uses,
                );
            }
            WordPart::CommandSubstitution { body, .. } => {
                collect_command_substitution_comparable_name_uses(
                    body,
                    semantic,
                    source,
                    allow_quoted_derived_words,
                    uses,
                );
            }
            WordPart::ArithmeticExpansion {
                expression_word_ast,
                ..
            } => {
                if let Some(name_use) = standalone_comparable_name_use(expression_word_ast, source)
                {
                    uses.push(name_use);
                }
            }
            WordPart::ParameterExpansion {
                operand_word_ast, ..
            }
            | WordPart::IndirectExpansion {
                operand_word_ast, ..
            } => {
                if let Some(word) = operand_word_ast {
                    collect_command_substitution_comparable_name_uses_in_parts(
                        &word.parts,
                        semantic,
                        source,
                        allow_quoted_derived_words,
                        uses,
                    );
                }
            }
            WordPart::Substring {
                offset_word_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                offset_word_ast,
                length_word_ast,
                ..
            } => {
                if let Some(name_use) = standalone_comparable_name_use(offset_word_ast, source) {
                    uses.push(name_use);
                }
                if let Some(word) = length_word_ast
                    && let Some(name_use) = standalone_comparable_name_use(word, source)
                {
                    uses.push(name_use);
                }
            }
            WordPart::Literal(_)
            | WordPart::ZshQualifiedGlob(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. } => {}
        }
    }
}

pub(crate) fn collect_command_substitution_comparable_name_uses(
    body: &StmtSeq,
    semantic: Option<&LinterSemanticArtifacts<'_>>,
    source: &str,
    allow_quoted_derived_words: bool,
    uses: &mut Vec<ComparableNameUse>,
) {
    let Some(semantic) = semantic else {
        return;
    };
    visit_command_substitution_candidate_words(body, semantic, source, &mut |word| {
        if !allow_quoted_derived_words
            && analyze_word(word, source, None).quote == WordQuote::FullyQuoted
        {
            return;
        }
        if let Some(mut name_use) = standalone_comparable_name_use(word, source) {
            name_use.mark_derived();
            uses.push(name_use);
        }
    });
}

pub(crate) fn standalone_comparable_name_use(
    word: &Word,
    source: &str,
) -> Option<ComparableNameUse> {
    if let Some(text) = static_word_text(word, source)
        && comparable_name_text(text.as_ref())
        && analyze_word(word, source, None).quote == WordQuote::Unquoted
    {
        return Some(literal_comparable_name_use(word.span, text.as_ref()));
    }

    standalone_comparable_parameter_name(&word.parts).map(|name| ComparableNameUse {
        span: word.span,
        key: ComparableNameKey(name.into()),
        kind: ComparableNameUseKind::Parameter,
    })
}

pub(crate) fn literal_comparable_name_use(span: Span, text: &str) -> ComparableNameUse {
    ComparableNameUse {
        span,
        key: ComparableNameKey(text.into()),
        kind: ComparableNameUseKind::Literal,
    }
}

pub(crate) fn comparable_name_uses_with_quoted_literals(
    word: &Word,
    semantic: Option<&LinterSemanticArtifacts<'_>>,
    source: &str,
) -> Box<[ComparableNameUse]> {
    let mut uses = comparable_name_uses(word, semantic, source).into_vec();
    if let Some(text) = static_word_text(word, source)
        && comparable_name_text(text.as_ref())
    {
        uses.push(literal_comparable_name_use(word.span, text.as_ref()));
    }
    dedup_comparable_name_uses(&mut uses);
    uses.into_boxed_slice()
}

pub(crate) fn standalone_comparable_parameter_name(parts: &[WordPartNode]) -> Option<&str> {
    match parts {
        [part] => comparable_name_from_word_part(part),
        _ => None,
    }
}

pub(crate) fn comparable_name_from_word_part(part: &WordPartNode) -> Option<&str> {
    match &part.kind {
        WordPart::Variable(name) if comparable_name_text(name.as_str()) => Some(name.as_str()),
        WordPart::Parameter(parameter) => comparable_name_from_parameter(parameter),
        WordPart::DoubleQuoted { parts, .. } => standalone_comparable_parameter_name(parts),
        _ => None,
    }
}

pub(crate) fn comparable_name_from_parameter(parameter: &ParameterExpansion) -> Option<&str> {
    match parameter.bourne()? {
        BourneParameterExpansion::Access { reference }
            if reference.subscript.is_none() && comparable_name_text(reference.name.as_str()) =>
        {
            Some(reference.name.as_str())
        }
        _ => None,
    }
}

pub(crate) fn comparable_name_text(text: &str) -> bool {
    is_shell_variable_name(text)
}

pub(crate) fn dedup_comparable_name_uses(uses: &mut Vec<ComparableNameUse>) {
    let mut seen = FxHashSet::<(ComparableNameKey, FactSpan)>::default();
    uses.retain(|name_use| seen.insert((name_use.key.clone(), FactSpan::new(name_use.span))));
}

pub(crate) fn heredoc_variable_name_span(span: Span, locator: Locator<'_>) -> Span {
    let source = locator.source();
    let Some(text) = source.get(span.start.offset..span.end.offset) else {
        return span;
    };
    let Some(relative_start) = text.find('$') else {
        return span;
    };
    let start_offset = span.start.offset + relative_start + '$'.len_utf8();
    let Some(start) = locator.position_at_offset(start_offset) else {
        return span;
    };
    Span::from_positions(start, span.end)
}

pub(crate) fn comparable_path_key_is_special_device(key: &ComparablePathKey) -> bool {
    let ComparablePathKey::Literal(path) = key else {
        return false;
    };

    matches!(
        path.as_ref(),
        "/dev/null" | "/dev/tty" | "/dev/stdin" | "/dev/stdout" | "/dev/stderr"
    ) || path
        .strip_prefix("/dev/fd/")
        .is_some_and(|suffix| suffix.bytes().all(|byte| byte.is_ascii_digit()))
        || path
            .strip_prefix("/proc/self/fd/")
            .is_some_and(|suffix| suffix.bytes().all(|byte| byte.is_ascii_digit()))
}

pub(crate) fn comparable_path_key_from_parts(
    parts: &[WordPartNode],
    source: &str,
) -> Option<ComparablePathKey> {
    let mut components = Vec::new();
    for part in parts {
        push_comparable_path_parts(part, source, &mut components)?;
    }

    match components.as_slice() {
        [ComparablePathPart::Literal(text)] => Some(ComparablePathKey::Literal(text.clone())),
        [ComparablePathPart::Parameter(name)] => Some(ComparablePathKey::Parameter(name.clone())),
        [] => None,
        _ => Some(ComparablePathKey::Template(components.into_boxed_slice())),
    }
}

pub(crate) fn push_comparable_path_parts(
    part: &WordPartNode,
    source: &str,
    components: &mut Vec<ComparablePathPart>,
) -> Option<()> {
    match &part.kind {
        WordPart::Literal(text) => {
            push_comparable_literal(text.as_str(source, part.span), components);
            Some(())
        }
        WordPart::SingleQuoted { value, .. } => {
            push_comparable_literal(value.slice(source), components);
            Some(())
        }
        WordPart::DoubleQuoted { parts, .. } => {
            for part in parts {
                push_comparable_path_parts(part, source, components)?;
            }
            Some(())
        }
        WordPart::Variable(name) if is_comparable_parameter_name(name.as_str()) => {
            components.push(ComparablePathPart::Parameter(name.as_str().into()));
            Some(())
        }
        WordPart::Variable(_) => None,
        WordPart::Parameter(parameter) => match parameter.bourne()? {
            BourneParameterExpansion::Access { reference }
                if reference.subscript.is_none()
                    && is_comparable_parameter_name(reference.name.as_str()) =>
            {
                components.push(ComparablePathPart::Parameter(
                    reference.name.as_str().into(),
                ));
                Some(())
            }
            _ => None,
        },
        WordPart::ZshQualifiedGlob(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
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
        | WordPart::Transformation { .. } => None,
    }
}

pub(crate) fn push_comparable_literal(text: &str, components: &mut Vec<ComparablePathPart>) {
    if text.is_empty() {
        return;
    }

    match components.last_mut() {
        Some(ComparablePathPart::Literal(existing)) => {
            let mut merged = existing.to_string();
            merged.push_str(text);
            *existing = merged.into_boxed_str();
        }
        _ => components.push(ComparablePathPart::Literal(text.into())),
    }
}

pub(crate) fn is_comparable_parameter_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[derive(Debug, Clone)]
pub struct RedirectFact<'a> {
    redirect: &'a Redirect,
    brace_fd_redirection_span: Option<Span>,
    operator_span: Span,
    target_span: Option<Span>,
    arithmetic_update_operator_spans: Box<[Span]>,
    analysis: Option<RedirectTargetAnalysis>,
    comparable_path: Option<ComparablePath>,
    comparable_name_uses: Box<[ComparableNameUse]>,
}

impl<'a> RedirectFact<'a> {
    pub fn redirect(&self) -> &'a Redirect {
        self.redirect
    }

    pub fn brace_fd_redirection_span(&self) -> Option<Span> {
        self.brace_fd_redirection_span
    }

    pub fn operator_span(&self) -> Span {
        self.operator_span
    }

    pub fn target_span(&self) -> Option<Span> {
        self.target_span
    }

    pub fn arithmetic_update_operator_spans(&self) -> &[Span] {
        &self.arithmetic_update_operator_spans
    }

    pub fn analysis(&self) -> Option<RedirectTargetAnalysis> {
        self.analysis
    }

    pub(crate) fn comparable_path(&self) -> Option<&ComparablePath> {
        self.comparable_path.as_ref()
    }

    pub(crate) fn comparable_name_uses(&self) -> &[ComparableNameUse] {
        &self.comparable_name_uses
    }
}

pub(crate) fn analyze_redirect_target(
    redirect: &Redirect,
    source: &str,
    behavior: Option<&ShellBehaviorAt<'_>>,
) -> Option<RedirectTargetAnalysis> {
    let target = redirect.word_target()?;
    let expansion = analyze_word(target, source, behavior);
    let runtime_literal = analyze_literal_runtime(
        target,
        source,
        ExpansionContext::RedirectTarget(redirect.kind),
        behavior,
    );

    let (kind, dev_null_status, numeric_descriptor_target) = match redirect.kind {
        RedirectKind::DupOutput | RedirectKind::DupInput => (
            RedirectTargetKind::DescriptorDup,
            None,
            static_word_text(target, source).and_then(|text| text.parse::<i32>().ok()),
        ),
        RedirectKind::HereDoc | RedirectKind::HereDocStrip => return None,
        RedirectKind::HereString
        | RedirectKind::Output
        | RedirectKind::Clobber
        | RedirectKind::Append
        | RedirectKind::Input
        | RedirectKind::ReadWrite
        | RedirectKind::OutputBoth => (
            RedirectTargetKind::File,
            Some(
                if static_word_text(target, source).as_deref() == Some("/dev/null") {
                    RedirectDevNullStatus::DefinitelyDevNull
                } else if expansion.is_fixed_literal() && !runtime_literal.is_runtime_sensitive() {
                    RedirectDevNullStatus::DefinitelyNotDevNull
                } else {
                    RedirectDevNullStatus::MaybeDevNull
                },
            ),
            None,
        ),
    };

    Some(RedirectTargetAnalysis {
        kind,
        dev_null_status,
        numeric_descriptor_target,
        expansion,
        runtime_literal,
    })
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(crate) fn build_redirect_facts<'a>(
    redirects: &'a [Redirect],
    semantic: Option<&LinterSemanticArtifacts<'a>>,
    locator: Locator<'_>,
    behavior: &ShellBehaviorAt<'_>,
) -> Vec<RedirectFact<'a>> {
    let source = locator.source();
    redirects
        .iter()
        .map(|redirect| RedirectFact {
            redirect,
            brace_fd_redirection_span: brace_fd_redirection_span(redirect, source),
            operator_span: redirect_operator_span(redirect),
            target_span: redirect.word_target().map(|word| word.span),
            arithmetic_update_operator_spans: redirect
                .word_target()
                .map_or_else(Vec::new, |word| {
                    let mut spans = Vec::new();
                    if let Some(semantic) = semantic {
                        collect_arithmetic_update_operator_spans_from_parts(
                            &word.parts,
                            semantic.semantic(),
                            source,
                            &mut spans,
                        );
                    }
                    spans
                })
                .into_boxed_slice(),
            analysis: analyze_redirect_target(redirect, source, Some(behavior)),
            comparable_path: redirect.word_target().and_then(|word| {
                ExpansionContext::from_redirect_kind(redirect.kind)
                    .and_then(|context| comparable_path(word, source, context, Some(behavior)))
            }),
            comparable_name_uses: comparable_redirect_name_uses(redirect, semantic, locator),
        })
        .collect()
}

pub(crate) fn comparable_redirect_name_uses(
    redirect: &Redirect,
    semantic: Option<&LinterSemanticArtifacts<'_>>,
    locator: Locator<'_>,
) -> Box<[ComparableNameUse]> {
    let source = locator.source();
    if let Some(word) = redirect.word_target() {
        return match redirect.kind {
            RedirectKind::Output
            | RedirectKind::Clobber
            | RedirectKind::Append
            | RedirectKind::OutputBoth => {
                comparable_name_uses_with_quoted_literals(word, semantic, source)
            }
            RedirectKind::Input
            | RedirectKind::ReadWrite
            | RedirectKind::HereDoc
            | RedirectKind::HereDocStrip
            | RedirectKind::HereString
            | RedirectKind::DupOutput
            | RedirectKind::DupInput => comparable_name_uses(word, semantic, source),
        };
    }

    let Some(heredoc) = redirect.heredoc() else {
        return Box::default();
    };
    if !heredoc.delimiter.expands_body {
        return Box::default();
    }
    comparable_heredoc_name_uses(&heredoc.body, semantic, locator)
}

pub(crate) fn brace_fd_redirection_span(redirect: &Redirect, source: &str) -> Option<Span> {
    let brace_span = redirect_fd_var_brace_span(redirect, source)?;
    let gap = source.get(brace_span.end.offset..redirect.span.start.offset)?;
    brace_fd_gap_allows_attachment(gap)
        .then(|| Span::from_positions(brace_span.start, redirect.span.end))
}

pub(crate) fn brace_fd_gap_allows_attachment(gap: &str) -> bool {
    if gap.is_empty() {
        return true;
    }

    let mut remaining = gap;
    while !remaining.is_empty() {
        if let Some(stripped) = remaining.strip_prefix("\\\r\n") {
            remaining = stripped;
            continue;
        }
        if let Some(stripped) = remaining.strip_prefix("\\\n") {
            remaining = stripped;
            continue;
        }
        return false;
    }

    true
}

pub(crate) fn redirect_operator_span(redirect: &Redirect) -> Span {
    let operator_start = redirect
        .fd_var_span
        .map(|span| span.end)
        .or_else(|| {
            redirect
                .fd
                .filter(|fd| *fd >= 0)
                .map(|fd| redirect.span.start.advanced_by(&fd.to_string()))
        })
        .unwrap_or(redirect.span.start);
    let operator_end = operator_start.advanced_by(redirect_operator_text(redirect.kind));

    Span::from_positions(operator_start, operator_end)
}

pub(crate) fn redirect_operator_text(kind: RedirectKind) -> &'static str {
    match kind {
        RedirectKind::Output => ">",
        RedirectKind::Clobber => ">|",
        RedirectKind::Append => ">>",
        RedirectKind::Input => "<",
        RedirectKind::ReadWrite => "<>",
        RedirectKind::HereDoc => "<<",
        RedirectKind::HereDocStrip => "<<-",
        RedirectKind::HereString => "<<<",
        RedirectKind::DupOutput => ">&",
        RedirectKind::DupInput => "<&",
        RedirectKind::OutputBoth => "&>",
    }
}

pub(super) fn duplicate_redirect_spans(redirects: &[RedirectFact<'_>], source: &str) -> Vec<Span> {
    let mut last_redirect_by_fd = FxHashMap::<i32, usize>::default();
    let mut consumed_redirects = FxHashSet::<usize>::default();
    let mut duplicate_redirects = FxHashSet::<usize>::default();

    for (index, redirect) in redirects.iter().enumerate() {
        for fd in redirect_read_fds(redirect, source) {
            if let Some(previous) = last_redirect_by_fd.get(&fd).copied() {
                consumed_redirects.insert(previous);
            }
        }

        for fd in duplicate_redirect_fds(redirect, source) {
            if let Some(previous) = last_redirect_by_fd.insert(fd, index)
                && !consumed_redirects.contains(&previous)
            {
                duplicate_redirects.insert(previous);
                duplicate_redirects.insert(index);
            }
        }
    }

    let mut spans = Vec::new();
    for (index, redirect) in redirects.iter().enumerate() {
        if !duplicate_redirects.contains(&index) {
            continue;
        }
        let Some(span) = duplicate_redirect_report_span(redirect, source) else {
            continue;
        };
        spans.push(span);
    }

    spans
}

fn duplicate_redirect_fds(redirect: &RedirectFact<'_>, source: &str) -> SmallVec<[i32; 2]> {
    let mut fds = SmallVec::new();
    let redirect_data = redirect.redirect();
    if redirect_data.fd_var.is_some() {
        return fds;
    }

    match redirect_data.kind {
        RedirectKind::Output | RedirectKind::Clobber | RedirectKind::Append => {
            if output_redirect_is_csh_both_target(redirect_data, source)
                || append_redirect_is_output_both(redirect_data, source)
            {
                if let Some(fd) = redirect_data.fd {
                    fds.push(fd);
                } else {
                    fds.push(1);
                    fds.push(2);
                }
            } else {
                fds.push(redirect_data.fd.unwrap_or(1));
            }
        }
        RedirectKind::Input
        | RedirectKind::ReadWrite
        | RedirectKind::HereDoc
        | RedirectKind::HereDocStrip
        | RedirectKind::HereString => fds.push(redirect_data.fd.unwrap_or(0)),
        RedirectKind::DupOutput if dup_output_redirects_to_file(redirect_data, source) => {
            if let Some(fd) = explicit_numeric_redirect_fd(redirect_data, source) {
                fds.push(fd);
            } else {
                fds.push(1);
                fds.push(2);
            }
        }
        RedirectKind::DupOutput if dup_redirects_to_descriptor(redirect_data, source) => {
            fds.push(redirect_data.fd.unwrap_or(1));
        }
        RedirectKind::DupInput if dup_redirects_to_descriptor(redirect_data, source) => {
            fds.push(redirect_data.fd.unwrap_or(0));
        }
        RedirectKind::OutputBoth => {
            fds.push(1);
            fds.push(2);
        }
        RedirectKind::DupOutput | RedirectKind::DupInput => {}
    }
    fds
}

fn redirect_read_fds(redirect: &RedirectFact<'_>, source: &str) -> SmallVec<[i32; 1]> {
    let mut fds = SmallVec::new();
    let redirect_data = redirect.redirect();
    if redirect_data.fd_var.is_some() {
        return fds;
    }

    match redirect_data.kind {
        RedirectKind::DupOutput | RedirectKind::DupInput => {
            if let Some(fd) = dup_redirect_descriptor_target(redirect_data, source) {
                fds.push(fd);
            }
        }
        RedirectKind::Output
        | RedirectKind::Clobber
        | RedirectKind::Append
        | RedirectKind::Input
        | RedirectKind::ReadWrite
        | RedirectKind::HereDoc
        | RedirectKind::HereDocStrip
        | RedirectKind::HereString
        | RedirectKind::OutputBoth => {}
    }

    fds
}

fn explicit_numeric_redirect_fd(redirect: &Redirect, source: &str) -> Option<i32> {
    let text = redirect.span.slice(source);
    let digit_len = text
        .bytes()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digit_len == 0 {
        return None;
    }

    text[..digit_len]
        .parse::<i32>()
        .ok()
        .filter(|fd| Some(*fd) == redirect.fd)
}

fn duplicate_redirect_report_span(redirect: &RedirectFact<'_>, source: &str) -> Option<Span> {
    match redirect.redirect().kind {
        RedirectKind::HereDoc | RedirectKind::HereDocStrip => {
            Some(redirect.redirect().heredoc()?.delimiter.span)
        }
        RedirectKind::OutputBoth => output_both_redirect_report_span(redirect.redirect(), source),
        RedirectKind::Output if output_redirect_is_csh_both_target(redirect.redirect(), source) => {
            csh_both_output_redirect_report_span(redirect, source)
        }
        RedirectKind::Output
        | RedirectKind::Clobber
        | RedirectKind::Append
        | RedirectKind::Input
        | RedirectKind::ReadWrite
        | RedirectKind::HereString => Some(redirect.operator_span()),
        RedirectKind::DupOutput if dup_output_redirects_to_file(redirect.redirect(), source) => {
            dup_output_file_redirect_report_span(redirect, source)
        }
        RedirectKind::DupOutput | RedirectKind::DupInput
            if dup_redirects_to_descriptor(redirect.redirect(), source) =>
        {
            Some(redirect.operator_span())
        }
        RedirectKind::DupOutput | RedirectKind::DupInput => None,
    }
}

fn dup_output_file_redirect_report_span(redirect: &RedirectFact<'_>, source: &str) -> Option<Span> {
    if explicit_numeric_redirect_fd(redirect.redirect(), source).is_some() {
        return Some(redirect.operator_span());
    }

    let text = redirect.redirect().span.slice(source);
    text.starts_with(">&").then(|| {
        Span::from_positions(
            redirect.redirect().span.start,
            redirect.redirect().span.start.advanced_by(">&"),
        )
    })
}

fn dup_output_redirects_to_file(redirect: &Redirect, source: &str) -> bool {
    let Some(target) = redirect.word_target() else {
        return false;
    };
    !static_word_text(target, source)
        .as_deref()
        .is_some_and(|text| text == "-" || text.parse::<i32>().is_ok())
}

fn dup_redirects_to_descriptor(redirect: &Redirect, source: &str) -> bool {
    dup_redirect_descriptor_target(redirect, source).is_some()
}

fn dup_redirect_descriptor_target(redirect: &Redirect, source: &str) -> Option<i32> {
    let target = redirect.word_target()?;
    static_word_text(target, source)
        .as_deref()
        .and_then(|text| text.parse::<i32>().ok())
}

fn output_redirect_is_csh_both_target(redirect: &Redirect, source: &str) -> bool {
    csh_both_output_redirect_span_and_target(redirect, source).is_some()
}

fn append_redirect_is_output_both(redirect: &Redirect, source: &str) -> bool {
    if redirect.kind != RedirectKind::Append || redirect.fd.is_some() {
        return false;
    }
    let Some(target) = redirect.word_target() else {
        return false;
    };
    let Some(prefix) = source.get(redirect.span.start.offset..target.span.start.offset) else {
        return false;
    };
    prefix
        .strip_prefix("&>>")
        .is_some_and(|gap| gap.bytes().all(|byte| byte.is_ascii_whitespace()))
}

fn csh_both_output_redirect_report_span(redirect: &RedirectFact<'_>, source: &str) -> Option<Span> {
    csh_both_output_redirect_span_and_target(redirect.redirect(), source).map(|(span, _)| span)
}

fn csh_both_output_redirect_span_and_target<'a>(
    redirect: &Redirect,
    source: &'a str,
) -> Option<(Span, &'a str)> {
    if redirect.kind != RedirectKind::Output {
        return None;
    }
    let target = redirect.word_target()?;
    let operator_start = redirect
        .fd
        .map(|fd| redirect.span.start.advanced_by(&fd.to_string()))
        .unwrap_or(redirect.span.start);
    let prefix = source.get(operator_start.offset..target.span.start.offset)?;

    let (span, target_text) = if prefix == ">&" {
        (
            Span::from_positions(operator_start, target.span.start),
            target.span.slice(source),
        )
    } else if prefix == ">" {
        let target_text = target.span.slice(source);
        (
            Span::from_positions(operator_start, target.span.start.advanced_by("&")),
            target_text.strip_prefix('&')?,
        )
    } else {
        return None;
    };

    (!target_text.is_empty() && target_text != "-" && target_text.parse::<i32>().is_err())
        .then_some((span, target_text))
}

fn output_both_redirect_report_span(redirect: &Redirect, source: &str) -> Option<Span> {
    let text = redirect.span.slice(source);
    let start_offset = text.find('>')?;
    let operator_text = &text[start_offset..];
    let width = operator_text
        .bytes()
        .take_while(|byte| *byte == b'>')
        .count()
        .max(1);
    let start = redirect.span.start.advanced_by(&text[..start_offset]);
    Some(Span::from_positions(
        start,
        start.advanced_by(&operator_text[..width]),
    ))
}
