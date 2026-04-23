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
pub(crate) enum ComparablePathPart {
    Literal(Box<str>),
    Parameter(Box<str>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ComparablePath {
    span: Span,
    key: ComparablePathKey,
}

impl ComparablePath {
    pub(crate) fn span(&self) -> Span {
        self.span
    }

    pub(crate) fn key(&self) -> &ComparablePathKey {
        &self.key
    }
}

pub(crate) fn comparable_path(
    word: &Word,
    source: &str,
    context: ExpansionContext,
    options: Option<&ZshOptionState>,
) -> Option<ComparablePath> {
    let analysis = analyze_word(word, source, options);
    if analysis.has_command_substitution()
        || analysis.hazards.command_or_process_substitution
        || analysis.has_array_expansion()
    {
        return None;
    }

    let runtime_literal = analyze_literal_runtime(word, source, context, options);
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
    })
}

fn comparable_path_key_is_special_device(key: &ComparablePathKey) -> bool {
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

fn comparable_path_key_from_parts(
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

fn push_comparable_path_parts(
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

fn push_comparable_literal(text: &str, components: &mut Vec<ComparablePathPart>) {
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

fn is_comparable_parameter_name(name: &str) -> bool {
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
}

pub(crate) fn analyze_redirect_target(
    redirect: &Redirect,
    source: &str,
    options: Option<&ZshOptionState>,
) -> Option<RedirectTargetAnalysis> {
    let target = redirect.word_target()?;
    let expansion = analyze_word(target, source, options);
    let runtime_literal = analyze_literal_runtime(
        target,
        source,
        ExpansionContext::RedirectTarget(redirect.kind),
        options,
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

fn build_redirect_facts<'a>(
    redirects: &'a [Redirect],
    source: &str,
    zsh_options: Option<&ZshOptionState>,
) -> Box<[RedirectFact<'a>]> {
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
                    collect_arithmetic_update_operator_spans_from_parts(
                        &word.parts,
                        source,
                        &mut spans,
                    );
                    spans
            })
            .into_boxed_slice(),
            analysis: analyze_redirect_target(redirect, source, zsh_options),
            comparable_path: redirect.word_target().and_then(|word| {
                ExpansionContext::from_redirect_kind(redirect.kind)
                    .and_then(|context| comparable_path(word, source, context, zsh_options))
            }),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn brace_fd_redirection_span(redirect: &Redirect, source: &str) -> Option<Span> {
    let brace_span = redirect_fd_var_brace_span(redirect, source)?;
    let gap = source.get(brace_span.end.offset..redirect.span.start.offset)?;
    brace_fd_gap_allows_attachment(gap)
        .then(|| Span::from_positions(brace_span.start, redirect.span.end))
}

fn brace_fd_gap_allows_attachment(gap: &str) -> bool {
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

fn redirect_operator_span(redirect: &Redirect) -> Span {
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

fn redirect_operator_text(kind: RedirectKind) -> &'static str {
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
