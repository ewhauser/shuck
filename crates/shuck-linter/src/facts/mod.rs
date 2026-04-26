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
mod misspelling;
mod normalized_commands;
mod presence;
pub(crate) mod surface;
#[cfg(test)]
mod tests;
#[allow(dead_code)]
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
        IndexedArrayReferenceFragmentFact, IndirectExpansionFragmentFact,
        NestedParameterExpansionFragmentFact, OpenDoubleQuoteFragmentFact,
        ParameterPatternSpecialTargetFragmentFact, PositionalParameterTrimFragmentFact,
        ReplacementExpansionFragmentFact, SubstringExpansionFragmentFact, SurfaceFragmentFacts,
        SurfaceFragmentSink, SurfaceScanContext, SuspectClosingQuoteFragmentFact,
        ZshParameterIndexFlagFragmentFact, build_subscript_later_suppression_reference_spans,
        build_suppressed_subscript_reference_spans, rewrite_pattern_as_single_double_quoted_string,
        rewrite_word_as_single_double_quoted_string,
    },
};
use crate::context::ContextRegionKind;
use crate::suppression::shellcheck_directive_can_apply_to_following_command;
use crate::{AmbientShellOptions, FileContext, ShellDialect};
use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    ArenaFile, ArenaFileCommandKind, ArenaHeredocBodyPart, ArithmeticExpansionSyntax,
    ArithmeticExpr, ArithmeticExprArena, ArithmeticExprArenaNode, ArithmeticExprNode,
    ArithmeticLvalue, ArithmeticLvalueArena, ArithmeticPostfixOp, ArithmeticUnaryOp, ArrayElem,
    ArrayElemNode, ArrayKind, Assignment, AssignmentNode, AssignmentValue, AssignmentValueNode,
    AstStore, BackgroundOperator, BinaryCommandView, BinaryOp, BourneParameterExpansion,
    BourneParameterExpansionNode, BraceQuoteContext, BraceSyntaxKind, BuiltinCommand, CaseCommand,
    CaseTerminator, Command, CommandId as AstCommandId, CommandSubstitutionSyntax, CommandView,
    CompoundCommand, CompoundCommandNode, ConditionalBinaryOp, ConditionalExpr,
    ConditionalExprArena, ConditionalUnaryOp, DeclOperand, DeclOperandNode, FunctionCommandView,
    FunctionDef, HeredocBodyPartNode, IdRange, ListArena, Name, ParameterExpansion,
    ParameterExpansionNode, ParameterExpansionSyntax, ParameterExpansionSyntaxNode, ParameterOp,
    Pattern, PatternNode, PatternPart, PatternPartArena, Position, PrefixMatchKind, Redirect,
    RedirectKind, RedirectNode, RedirectTargetNode, SimpleCommand, SourceText, Span,
    StaticCommandWrapperTarget, Stmt, StmtId as AstStmtId, StmtSeq, StmtSeqView, StmtTerminator,
    StmtView, Subscript, SubscriptNode, SubscriptSelector, TextRange, TextSize, VarRef, VarRefNode,
    WhileCommand, Word, WordId, WordPart, WordPartArena, WordPartArenaNode, WordPartNode, WordView,
    ZshExpansionOperation, ZshExpansionOperationNode, ZshExpansionTarget, ZshExpansionTargetNode,
    ZshGlobSegment, ZshGlobSegmentNode, ZshQualifiedGlob, is_shell_variable_name,
    static_command_name_text, static_command_name_text_arena, static_command_wrapper_target_index,
    static_word_text, static_word_text_arena, word_is_standalone_status_capture,
};
use shuck_indexer::Indexer;
use shuck_parser::parser::Parser;
use shuck_semantic::{
    Binding, BindingAttributes, BindingId, BindingKind, DeclarationBuiltin, OptionValue, Reference,
    ReferenceId, ReferenceKind, ScopeId, SemanticModel, ZshOptionState,
};
use smallvec::SmallVec;
use std::{borrow::Cow, collections::VecDeque, ops::ControlFlow, sync::OnceLock};

pub use self::conditional_portability::ConditionalPortabilityFacts;
pub(crate) use self::escape_scan::{EscapeScanMatch, EscapeScanSourceKind};
pub use self::normalized_commands::{
    ArenaNormalizedCommand, ArenaNormalizedDeclaration, DeclarationKind, NormalizedCommand,
    NormalizedDeclaration, WrapperKind,
};
pub use self::surface::{
    BacktickFragmentFact, LegacyArithmeticFragmentFact, PositionalParameterFragmentFact,
    SingleQuotedFragmentFact,
};

include!("traversal.rs");
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

#[cfg(feature = "benchmarking")]
pub(crate) fn benchmark_normalize_commands(arena_file: &ArenaFile, source: &str) -> usize {
    let mut total = 0;
    walk_arena_commands(
        arena_file.view().body(),
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |visit, _| {
            let normalized = command::normalize_arena_command(visit.command, source);
            total += normalized.wrappers.len()
                + normalized.body_words.len()
                + usize::from(normalized.literal_name.is_some())
                + usize::from(normalized.effective_name.is_some())
                + usize::from(normalized.declaration.is_some());
        },
    );
    total
}

#[allow(unused_imports)]
pub(crate) mod core {
    pub use super::{
        CommandFactRef, CommandFacts, CommandId, FactSpan, FactWordSpan, SudoFamilyInvoker,
        WordNodeId, WordOccurrenceId,
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
    pub use super::{CommandFact, CommandFactCompoundKind, CommandFactRef};
}

#[allow(unused_imports)]
pub(crate) mod words {
    pub use super::{
        FactWordRef, WordFactContext, WordFactHostKind, WordOccurrence, WordOccurrenceIter,
        WordOccurrenceRef, leading_literal_word_prefix,
    };
}
