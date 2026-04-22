//! Linter-owned structural facts built once per file.
//!
//! `SemanticModel` remains the source of truth for bindings, references, scopes,
//! source references, the call graph, and flow-sensitive facts.
//! `LinterFacts` owns reusable linter-side summaries that are cheaper to build
//! once than to recompute in every rule: normalized commands, wrapper chains,
//! declaration summaries, option-shape summaries, and later word/expansion
//! facts.

mod conditional_portability;
mod escape_scan;
mod presence;
pub(crate) mod surface;
#[cfg(test)]
mod tests;

use self::{
    conditional_portability::{ConditionalPortabilityInputs, build_conditional_portability_facts},
    escape_scan::{EscapeScanContext, EscapeScanInputs, build_escape_scan_matches},
    presence::build_presence_tested_names,
    surface::{
        CaseModificationFragmentFact, CasePatternExpansionFact,
        CommandSubstitutionParameterOperationFragmentFact, DollarDoubleQuotedFragmentFact,
        IndexedArrayReferenceFragmentFact, IndirectExpansionFragmentFact,
        NestedParameterExpansionFragmentFact, OpenDoubleQuoteFragmentFact,
        ParameterPatternSpecialTargetFragmentFact, ReplacementExpansionFragmentFact,
        StarGlobRemovalFragmentFact, SubstringExpansionFragmentFact, SurfaceFragmentFacts,
        SurfaceFragmentSink, SurfaceScanContext, SuspectClosingQuoteFragmentFact,
        ZshParameterIndexFlagFragmentFact, build_subscript_index_reference_spans,
        rewrite_pattern_as_single_double_quoted_string,
        rewrite_word_as_single_double_quoted_string,
    },
};
use crate::context::ContextRegionKind;
use crate::rules::common::expansion::{
    ExpansionAnalysis, ExpansionContext, RedirectTargetAnalysis, RuntimeLiteralAnalysis,
    SubstitutionOutputIntent, WordExpansionKind, WordLiteralness, WordSubstitutionShape,
    analyze_literal_runtime, analyze_redirect_target, analyze_word,
};
use crate::rules::common::span::{
    expansion_part_spans, word_unbraced_variable_before_bracket_spans,
};
use crate::rules::common::{
    command::{self, DeclarationKind, NormalizedCommand, NormalizedDeclaration, WrapperKind},
    query::{self, CommandSubstitutionKind, CommandVisit, CommandWalkOptions},
    span,
    word::{
        TestOperandClass, WordClassification, WordQuote, classify_conditional_operand,
        classify_contextual_operand, classify_word, static_word_text,
    },
};
use crate::suppression::shellcheck_directive_can_apply_to_following_command;
use crate::{AmbientShellOptions, FileContext};
use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    ArithmeticExpansionSyntax, ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue,
    ArithmeticPostfixOp, ArithmeticUnaryOp, ArrayElem, ArrayKind, Assignment, AssignmentValue,
    BackgroundOperator, BinaryCommand, BinaryOp, BourneParameterExpansion, BraceQuoteContext,
    BraceSyntaxKind, BuiltinCommand, CaseCommand, CaseItem, CaseTerminator, Command,
    CommandSubstitutionSyntax, CompoundCommand, ConditionalBinaryOp, ConditionalExpr,
    ConditionalUnaryOp, DeclClause, DeclOperand, File, ForCommand, FunctionDef, IfCommand, Name,
    ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Pattern, PatternPart, Position,
    Redirect, RedirectKind, SelectCommand, SimpleCommand, SourceText, Span, Stmt, StmtSeq,
    StmtTerminator, Subscript, TextRange, VarRef, WhileCommand, Word, WordPart, WordPartNode,
    ZshExpansionTarget, ZshGlobSegment, ZshQualifiedGlob,
};
use shuck_indexer::Indexer;
use shuck_parser::parser::Parser;
use shuck_semantic::{
    BindingAttributes, BindingId, BindingKind, ScopeId, SemanticModel, ZshOptionState,
};
use smallvec::SmallVec;
use std::{borrow::Cow, cell::OnceCell, ops::ControlFlow};

pub use self::conditional_portability::ConditionalPortabilityFacts;
pub(crate) use self::escape_scan::{EscapeScanMatch, EscapeScanSourceKind};
pub use self::surface::{
    BacktickFragmentFact, LegacyArithmeticFragmentFact, PositionalParameterFragmentFact,
    SingleQuotedFragmentFact,
};

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
include!("command_options.rs");
include!("commands.rs");
include!("model.rs");
include!("builder.rs");
include!("assignments.rs");
include!("arrays.rs");
include!("comments.rs");
include!("heredocs.rs");
include!("braces.rs");
include!("arithmetic.rs");
include!("words.rs");

#[allow(unused_imports)]
pub(crate) mod core {
    pub use super::{CommandId, FactSpan, SudoFamilyInvoker, WordNodeId, WordOccurrenceId};
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
    pub use super::{SubstitutionFact, SubstitutionHostKind};
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
pub(crate) mod command_options {
    pub use super::{
        CommandOptionFacts, ExitCommandFacts, FindCommandFacts, FindExecCommandFacts,
        FindExecShellCommandFacts, GrepPatternSourceKind, PathWordFact, PrintfCommandFacts,
        ReadCommandFacts, RmCommandFacts, SshCommandFacts, SudoFamilyCommandFacts,
        UnsetCommandFacts, WaitCommandFacts, XargsCommandFacts,
    };
}

#[allow(unused_imports)]
pub(crate) mod commands {
    pub use super::CommandFact;
}

#[allow(unused_imports)]
pub(crate) mod words {
    pub use super::{
        WordFactContext, WordFactHostKind, WordOccurrence, WordOccurrenceIter, WordOccurrenceRef,
        leading_literal_word_prefix,
    };
}
