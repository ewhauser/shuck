//! Linter-owned structural facts built once per file.
//!
//! `SemanticModel` remains the source of truth for bindings, references, scopes,
//! source references, the call graph, and flow-sensitive facts.
//! `LinterFacts` owns reusable linter-side summaries that are cheaper to build
//! once than to recompute in every rule: normalized commands, wrapper chains,
//! declaration summaries, option-shape summaries, and later word/expansion
//! facts.

pub(crate) mod command_options;
mod conditional_portability;
mod escape_scan;
mod misspelling;
mod normalized_commands;
mod presence;
pub(crate) mod surface;
#[cfg(test)]
mod tests;
pub(crate) mod word_spans;

use self::word_spans::{expansion_part_spans, word_unbraced_variable_before_bracket_spans};
use self::{
    conditional_portability::{ConditionalPortabilityInputs, build_conditional_portability_facts},
    escape_scan::{EscapeScanContext, EscapeScanInputs, build_escape_scan_matches},
    misspelling::{
        PossibleVariableMisspellingIndex, build_possible_variable_misspelling_index,
        scan_possible_variable_misspelling_candidate,
        should_scan_possible_variable_misspelling_candidates,
    },
    normalized_commands as command,
    presence::{PresenceTestNameFact, PresenceTestReferenceFact, build_presence_tested_names},
    surface::{
        CaseModificationFragmentFact, CasePatternExpansionFact, DollarDoubleQuotedFragmentFact,
        IndirectExpansionFragmentFact, NestedParameterExpansionFragmentFact,
        OpenDoubleQuoteFragmentFact, ParameterPatternSpecialTargetFragmentFact,
        PositionalParameterTrimFragmentFact, ReplacementExpansionFragmentFact,
        SubstringExpansionFragmentFact, SurfaceFragmentFacts, SurfaceFragmentSink,
        SurfaceScanContext, SuspectClosingQuoteFragmentFact, ZshParameterIndexFlagFragmentFact,
        build_subscript_later_suppression_reference_spans,
        build_suppressed_subscript_reference_spans, rewrite_pattern_as_single_double_quoted_string,
        rewrite_word_as_single_double_quoted_string,
    },
};
use crate::{
    AmbientShellOptions, CommandTopology, CommandTopologyTraversal, LinterSemanticArtifacts,
    Locator, ShellDialect, WordQuote,
};
use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    ArithmeticCommand, ArithmeticExpansionSyntax, ArithmeticExpr, ArithmeticExprNode,
    ArithmeticLvalue, ArithmeticPostfixOp, ArithmeticUnaryOp, ArrayElem, ArrayKind, Assignment,
    AssignmentValue, BackgroundOperator, BinaryCommand, BinaryOp, BourneParameterExpansion,
    BraceQuoteContext, BraceSyntaxKind, BuiltinCommand, CaseCommand, CaseItem, CaseTerminator,
    Command, CommandSubstitutionSyntax, CompoundCommand, ConditionalBinaryOp, ConditionalExpr,
    ConditionalUnaryOp, DeclClause, DeclOperand, File, ForCommand, FunctionDef, IdRange, ListArena,
    Name, ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Pattern, PatternPart,
    Position, PrefixMatchKind, Redirect, RedirectKind, SelectCommand, SimpleCommand, SourceText,
    Span, StaticCommandWrapperTarget, Stmt, StmtSeq, StmtTerminator, Subscript, SubscriptSelector,
    TextRange, TextSize, VarRef, WhileCommand, Word, WordPart, WordPartNode, ZshExpansionOperation,
    ZshExpansionTarget, ZshGlobSegment, ZshParameterExpansion, ZshQualifiedGlob,
    is_shell_variable_name, static_command_name_text, static_command_wrapper_target_index,
    static_word_text, word_is_standalone_status_capture,
};
use shuck_indexer::{Indexer, LineIndex};
#[cfg(test)]
use shuck_parser::parser::Parser;
use shuck_semantic::{
    ArithmeticLiteralBehavior, Binding, BindingAttributes, BindingId, BindingKind, CaseCliDispatch,
    Declaration, DeclarationBuiltin, DeclarationOperand as SemanticDeclarationOperand,
    FieldSplittingBehavior, FileExpansionOrderBehavior, GlobDotBehavior, GlobFailureBehavior,
    GlobPatternBehavior, NonpersistentAssignmentAnalysisContext,
    NonpersistentAssignmentAnalysisOptions, NonpersistentAssignmentCommandContext,
    NonpersistentAssignmentExtraRead, OptionValue, PathnameExpansionBehavior,
    PatternOperatorBehavior, Reference, ReferenceId, ReferenceKind, ScopeId, SemanticAnalysis,
    SemanticModel, SemanticPipelineOperatorKind, SubscriptIndexBehavior, ZshOptionState,
};
use smallvec::SmallVec;
use std::{borrow::Cow, ops::ControlFlow, sync::OnceLock};

#[allow(unused_imports)]
pub(crate) use self::command_options::{
    CommandOptionFacts, ExitCommandFacts, ExprCommandFacts, ExprStringHelperKind,
    FunctionPositionalParameterFacts, GrepPatternSourceKind, MapfileCommandFacts, PathWordFact,
    WaitCommandFacts,
};
#[allow(unused_imports)]
pub(crate) use self::command_options::{
    SedScriptQuoteMode, UnsetArraySubscriptFact, UnsetOperandFact, XargsShortOptionArgumentStyle,
    find_sed_substitution_section, sed_has_single_substitution_script, sed_script_text,
    shell_flag_contains_command_string, short_option_cluster_contains_flag,
    ssh_option_consumes_next_argument, word_starts_with_literal_dash,
    xargs_long_option_requires_separate_argument, xargs_short_option_argument_style,
};
pub use self::conditional_portability::ConditionalPortabilityFacts;
pub(crate) use self::escape_scan::{EscapeScanMatch, EscapeScanSourceKind};
pub use self::normalized_commands::{
    DeclarationKind, NormalizedCommand, NormalizedDeclaration, WrapperKind,
};
pub use self::surface::{
    AmbiguousArrayReference, ArithmeticLiteralFact, BacktickFragmentFact,
    IndexedArrayReferenceFragment, IndexedArrayReferenceFragmentFact, LegacyArithmeticFragmentFact,
    NativeZshScalarArrayReference, PlainUnindexedArrayReferenceFact,
    PositionalParameterFragmentFact, SelectorRequiredArrayReference, SingleQuotedFragmentFact,
};

include!("traversal.rs");
include!("body_shape.rs");
include!("core.rs");
include!("simple_tests.rs");
include!("conditionals.rs");
include!("redirects.rs");
include!("substitutions.rs");
include!("loop_headers.rs");
include!("case_patterns.rs");
include!("functions.rs");
include!("pipelines.rs");
include!("lists.rs");
include!("statements.rs");
include!("commands.rs");
include!("model.rs");
include!("builder.rs");
include!("assignments.rs");
include!("arrays.rs");
include!("comments.rs");
include!("heredocs.rs");
include!("braces.rs");
include!("arithmetic.rs");
pub(crate) mod words;
use self::words::*;

#[allow(unused_imports)]
pub(crate) mod core {
    pub use super::{
        CommandFactRef, CommandFacts, CommandId, FactSpan, SudoFamilyInvoker, WordNodeId,
        WordOccurrenceId,
    };
}

#[allow(unused_imports)]
pub(crate) mod simple_tests {
    pub use super::{SimpleTestFact, SimpleTestOperatorFamily, SimpleTestShape, SimpleTestSyntax};
}

#[allow(unused_imports)]
pub(crate) mod conditionals {
    pub use super::{
        ConditionalBareWordFact, ConditionalBinaryFact, ConditionalFact,
        ConditionalMixedLogicalOperatorFact, ConditionalNodeFact, ConditionalOperandFact,
        ConditionalOperatorFamily, ConditionalUnaryFact,
    };
}

#[allow(unused_imports)]
pub(crate) mod redirects {
    pub use super::RedirectFact;
}

#[allow(unused_imports)]
pub(crate) mod substitutions {
    pub use super::{CommandSubstitutionKind, SubstitutionFact, SubstitutionHostKind};
}

#[allow(unused_imports)]
pub(crate) mod loop_headers {
    pub use super::{ForHeaderFact, LoopHeaderWordFact, SelectHeaderFact};
}

#[allow(unused_imports)]
pub(crate) mod functions {
    pub use super::{FunctionCallArityFacts, FunctionHeaderFact};
}

#[allow(unused_imports)]
pub(crate) mod pipelines {
    pub use super::{PipelineFact, PipelineOperatorFact, PipelineSegmentFact};
}

#[allow(unused_imports)]
pub(crate) mod lists {
    pub use super::{ListFact, ListOperatorFact, ListSegmentKind, MixedShortCircuitKind};
}

#[allow(unused_imports)]
pub(crate) mod statements {
    pub use super::StatementFact;
}

#[allow(unused_imports)]
pub(crate) mod commands {
    pub use super::{CommandFact, CommandFactRef};
}
