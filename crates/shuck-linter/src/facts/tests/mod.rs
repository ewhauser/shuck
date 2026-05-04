#[allow(unused_imports)]
use shuck_ast::{BinaryOp, CommandSubstitutionSyntax, ConditionalBinaryOp, Name};
use shuck_indexer::Indexer;
use shuck_parser::parser::{Parser, ShellDialect as ParseShellDialect};
use shuck_semantic::BindingAttributes;
use std::path::Path;

#[allow(unused_imports)]
use super::{
    CommandId, ConditionalNodeFact, ConditionalOperatorFamily, ExpansionContext,
    ExprStringHelperKind, GrepPatternSourceKind, ListFact, ListSegmentKind, MixedShortCircuitKind,
    SimpleTestOperatorFamily, SimpleTestShape, SimpleTestSyntax, SubstitutionHostKind,
    SubstitutionOutputIntent, SudoFamilyInvoker, WordFactHostKind,
    build_innermost_command_ids_by_offset, precomputed_command_id_for_offset,
};
use crate::WrapperKind;
use crate::facts::surface::PositionalParameterFragmentKind;
use crate::{ArithmeticLiteralBehavior, LinterFacts, LinterSemanticArtifacts, ShellDialect};

mod assignments;
mod braces;
mod commands;
mod comments;
mod conditions;
mod flow;
mod functions;
mod support;
mod surface;

use support::{with_facts, with_facts_dialect};
