use compact_str::CompactString;
use shuck_ast::{Name, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRefDiagnosticClass {
    DynamicPath,
    UntrackedFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    pub kind: SourceRefKind,
    pub span: Span,
    pub path_span: Span,
    pub resolution: SourceRefResolution,
    pub explicitly_provided: bool,
    pub diagnostic_class: SourceRefDiagnosticClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRefKind {
    Literal(CompactString),
    Directive(CompactString),
    DirectiveDevNull,
    Dynamic,
    SingleVariableStaticTail { variable: Name, tail: CompactString },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRefResolution {
    Unchecked,
    Resolved,
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
