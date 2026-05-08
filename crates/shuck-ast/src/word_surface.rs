//! Parser-owned word surface syntax adjuncts.
//!
//! These types preserve word-level source syntax that downstream consumers may
//! want to inspect without re-scanning raw source text. They complement the
//! main [`Word`](crate::Word) AST rather than replacing it.

use crate::ast::{BraceQuoteContext, BraceSyntax};
use crate::{Name, Span};

/// Parser-owned, dialect-neutral surface syntax attached to a word.
#[derive(Debug, Clone, Default)]
pub struct WordSurfaceSyntax {
    /// Brace-like surface syntax occurrences, including literal and
    /// template-placeholder braces.
    pub braces: Vec<BraceSyntax>,
    /// Escaped `\${...}` template bodies preserved from the original source.
    pub escaped_parameter_templates: Vec<EscapedParameterTemplateSyntax>,
    /// All-elements array-expansion surfaces such as `$@`, `${@}`, or
    /// `${array[@]}`.
    pub all_elements_array_expansions: Vec<AllElementsArrayExpansionSyntax>,
}

impl WordSurfaceSyntax {
    /// Borrow the brace-like surface syntax entries.
    pub fn braces(&self) -> &[BraceSyntax] {
        &self.braces
    }

    /// Borrow escaped parameter-template entries.
    pub fn escaped_parameter_templates(&self) -> &[EscapedParameterTemplateSyntax] {
        &self.escaped_parameter_templates
    }

    /// Borrow all-elements array-expansion entries.
    pub fn all_elements_array_expansions(&self) -> &[AllElementsArrayExpansionSyntax] {
        &self.all_elements_array_expansions
    }

    /// Iterate over all dialect-neutral surface syntax entries.
    pub fn iter(&self) -> impl Iterator<Item = WordSurfaceSyntaxRef<'_>> {
        self.braces
            .iter()
            .map(WordSurfaceSyntaxRef::Brace)
            .chain(
                self.escaped_parameter_templates
                    .iter()
                    .map(WordSurfaceSyntaxRef::EscapedParameterTemplate),
            )
            .chain(
                self.all_elements_array_expansions
                    .iter()
                    .map(WordSurfaceSyntaxRef::AllElementsArrayExpansion),
            )
    }

    /// Returns whether this word has no dialect-neutral surface syntax.
    pub fn is_empty(&self) -> bool {
        self.braces.is_empty()
            && self.escaped_parameter_templates.is_empty()
            && self.all_elements_array_expansions.is_empty()
    }
}

/// Parser-owned zsh-only surface syntax attached to a word.
#[derive(Debug, Clone, Default)]
pub struct ZshWordSurfaceSyntax {
    /// zsh short positional-parameter surfaces such as `@[1]` or `@[1,3]`.
    pub short_positional_at: Vec<ZshShortPositionalAtSyntax>,
}

impl ZshWordSurfaceSyntax {
    /// Borrow short positional-parameter entries.
    pub fn short_positional_at(&self) -> &[ZshShortPositionalAtSyntax] {
        &self.short_positional_at
    }

    /// Iterate over all zsh-only surface syntax entries.
    pub fn iter(&self) -> impl Iterator<Item = ZshWordSurfaceSyntaxRef<'_>> {
        self.short_positional_at
            .iter()
            .map(ZshWordSurfaceSyntaxRef::ShortPositionalAt)
    }

    /// Returns whether this word has no zsh-only surface syntax.
    pub fn is_empty(&self) -> bool {
        self.short_positional_at.is_empty()
    }
}

/// A borrowed dialect-neutral word-surface syntax entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordSurfaceSyntaxRef<'a> {
    /// A brace-like surface syntax occurrence.
    Brace(&'a BraceSyntax),
    /// An escaped `\${...}` template occurrence.
    EscapedParameterTemplate(&'a EscapedParameterTemplateSyntax),
    /// An all-elements array-expansion occurrence.
    AllElementsArrayExpansion(&'a AllElementsArrayExpansionSyntax),
}

/// A borrowed zsh-only word-surface syntax entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZshWordSurfaceSyntaxRef<'a> {
    /// A zsh short positional-parameter occurrence.
    ShortPositionalAt(&'a ZshShortPositionalAtSyntax),
}

/// An escaped `\${...}` template occurrence inside a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EscapedParameterTemplateSyntax {
    /// Full surface span covering the escaped template, including `\${` and
    /// the closing `}`.
    pub span: Span,
    /// Interior body span excluding the leading `\${` and trailing `}`.
    pub body_span: Span,
    /// Quoting context that governed this surface syntax.
    pub quote_context: BraceQuoteContext,
    /// Whether the body contains a nested `${...}` fragment.
    pub contains_nested_parameter: bool,
}

/// Kind of all-elements array expansion represented by a surface span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllElementsArrayExpansionKind {
    /// Positional-parameter `@` expansion such as `$@` or `${@}`.
    PositionalAt,
    /// Positional-parameter `*` expansion such as `$*` or `${*}`.
    PositionalStar,
    /// Selector-based `[@]` expansion such as `${array[@]}`.
    SelectorAt,
    /// Selector-based `[*]` expansion such as `${array[*]}`.
    SelectorStar,
}

/// Where an all-elements array-expansion surface came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllElementsArrayExpansionOrigin {
    /// The surface syntax was represented directly by a parsed word part.
    DirectPart,
    /// The surface syntax was recovered from nested text inside another
    /// expansion body.
    NestedParameterBody,
}

/// A parser-owned all-elements array-expansion surface span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllElementsArrayExpansionSyntax {
    /// Source span covering the surface syntax occurrence.
    pub span: Span,
    /// Shape of the all-elements expansion.
    pub kind: AllElementsArrayExpansionKind,
    /// Whether the syntax was direct or recovered from nested parameter text.
    pub origin: AllElementsArrayExpansionOrigin,
    /// Whether this surface counts as a direct all-elements expansion for
    /// direct-only linter checks.
    pub direct: bool,
    /// Quoting context that governed this surface syntax.
    pub quote_context: BraceQuoteContext,
}

/// Kind of parser-owned unquoted named-reference candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnquotedReferenceCandidateKind {
    /// A variable part such as `$foo` or `$@`.
    Variable,
    /// A parameter access such as `${foo}`.
    ParameterAccess,
}

/// A parser-owned top-level unquoted named-reference candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnquotedReferenceCandidateRef<'a> {
    /// Source span covering the full surface occurrence.
    pub span: Span,
    /// Source span that should be used for semantic reference lookup.
    pub lookup_span: Span,
    /// Referenced name.
    pub name: &'a Name,
    /// Surface flavor of the reference candidate.
    pub kind: UnquotedReferenceCandidateKind,
}

/// Kind of zsh short positional-parameter syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZshShortPositionalAtKind {
    /// Indexed short positional syntax such as `@[1]`.
    IndexedSubscript,
    /// Range-like short positional syntax such as `@[1,3]`.
    Range,
}

/// A zsh short positional-parameter surface span such as `@[1]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZshShortPositionalAtSyntax {
    /// Full source span covering the `@[...]` surface.
    pub span: Span,
    /// Source span of the leading `@`.
    pub base_span: Span,
    /// Source span of the trailing `[ ... ]` suffix.
    pub suffix_span: Span,
    /// Flavor of the zsh-only short positional syntax.
    pub kind: ZshShortPositionalAtKind,
    /// Quoting context that governed this surface syntax.
    pub quote_context: BraceQuoteContext,
}
