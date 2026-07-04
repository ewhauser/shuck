use compact_str::CompactString;
use shuck_ast::{Name, Span};

/// Which shuck-native source hint, if any, annotated a `source` reference.
///
/// Distinguishes the shuck-native directives from the ShellCheck-compatible
/// `# shellcheck source=` directive (which keeps ShellCheck's louder
/// not-specified-as-input semantics) and from non-directive references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceHint {
    /// No shuck-native hint: a plain reference or `# shellcheck source=`.
    #[default]
    None,
    /// `# shuck: assume-source=<path>` — trust the path and import its symbols.
    Assume,
    /// `# shuck: follow-source=<path>` — trust and import, and lint/follow the target.
    Follow,
}

impl SourceHint {
    /// Whether this hint is a shuck-native assertion (`assume`/`follow`).
    pub fn is_native(self) -> bool {
        matches!(self, SourceHint::Assume | SourceHint::Follow)
    }

    /// Whether the hint requests following (linting) the target.
    pub fn follows(self) -> bool {
        matches!(self, SourceHint::Follow)
    }
}

/// Broad diagnostic family for a discovered `source`-style reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRefDiagnosticClass {
    /// The path is dynamic enough that static resolution is not reliable.
    DynamicPath,
    /// The path is statically identifiable but may not be tracked by the build.
    UntrackedFile,
}

/// One semantic `source` or `.` reference discovered in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    /// Syntactic shape of the referenced path.
    pub kind: SourceRefKind,
    /// Span of the full `source` command or relevant operand.
    pub span: Span,
    /// Span of the path-like portion being resolved.
    pub path_span: Span,
    /// Resolution status computed for this reference.
    pub resolution: SourceRefResolution,
    /// Whether the path came from an explicitly provided source directive.
    pub explicitly_provided: bool,
    /// Which shuck-native source hint (if any) annotated this reference.
    pub hint: SourceHint,
    /// Diagnostic family higher layers should use for unresolved cases.
    pub diagnostic_class: SourceRefDiagnosticClass,
}

/// Encoded path form for a source-like reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRefKind {
    /// A fully static literal path.
    Literal(CompactString),
    /// A path injected by a source directive.
    Directive(CompactString),
    /// A source directive that intentionally resolves to `/dev/null`.
    DirectiveDevNull,
    /// A path whose value is too dynamic to resolve statically.
    Dynamic,
    /// A path built from one variable plus a static tail suffix.
    SingleVariableStaticTail {
        /// Variable that contributes the dynamic prefix.
        variable: Name,
        /// Static suffix appended after the variable value.
        tail: CompactString,
    },
}

/// Whether a source-like reference resolved to a tracked target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRefResolution {
    /// Resolution was not attempted.
    Unchecked,
    /// Resolution succeeded.
    Resolved,
    /// Resolution was attempted but no tracked target was found.
    Unresolved,
}

pub(crate) fn default_diagnostic_class(kind: &SourceRefKind) -> SourceRefDiagnosticClass {
    match kind {
        SourceRefKind::DirectiveDevNull | SourceRefKind::Directive(_) => {
            SourceRefDiagnosticClass::UntrackedFile
        }
        SourceRefKind::Literal(_) => SourceRefDiagnosticClass::UntrackedFile,
        SourceRefKind::Dynamic => SourceRefDiagnosticClass::DynamicPath,
        SourceRefKind::SingleVariableStaticTail { tail, .. } => {
            if tail.starts_with('/') {
                SourceRefDiagnosticClass::UntrackedFile
            } else {
                SourceRefDiagnosticClass::DynamicPath
            }
        }
    }
}
