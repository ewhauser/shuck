use compact_str::CompactString;
use shuck_ast::{Name, Span};

/// Which spelling produced an explicit source directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceDirectiveOrigin {
    /// `# shuck: source=<path>` — the shuck-native spelling. A resolved
    /// target is an explicit user assertion and silences the untracked-source
    /// diagnostics at the site.
    Shuck,
    /// `# shellcheck source=<path>` — the ShellCheck-compatible spelling,
    /// which keeps ShellCheck's louder not-specified-as-input semantics.
    ShellCheck,
}

/// An explicit source directive annotating a `source` reference.
///
/// The asserted target itself lives in [`SourceRefKind::Directive`] (or
/// [`SourceRefKind::DirectiveDevNull`]); this carries the directive's origin
/// and its lint policy. A plain, un-annotated reference has no
/// `SourceDirectiveInfo` at all, so "no directive" is not conflated with any
/// particular directive spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceDirectiveInfo {
    /// Which spelling produced the directive.
    pub origin: SourceDirectiveOrigin,
    /// Whether the directive asks for the target to be linted as an
    /// additional input (`lint=true`). Defaults to false: assert the target
    /// and import its symbols only.
    pub lint: bool,
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
    /// The explicit directive (if any) that annotated this reference.
    pub directive: Option<SourceDirectiveInfo>,
    /// Diagnostic family higher layers should use for unresolved cases.
    pub diagnostic_class: SourceRefDiagnosticClass,
}

impl SourceRef {
    /// Whether a shuck-native `# shuck: source=` directive annotated this
    /// reference (as opposed to the ShellCheck-compatible spelling or no
    /// directive at all).
    pub fn has_shuck_directive(&self) -> bool {
        self.directive
            .is_some_and(|directive| directive.origin == SourceDirectiveOrigin::Shuck)
    }

    /// Whether the annotating directive asks for the target to be linted as
    /// an additional input (`lint=true`).
    pub fn lints_target(&self) -> bool {
        self.directive.is_some_and(|directive| directive.lint)
    }
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
