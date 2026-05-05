use shuck_ast::{
    ArithmeticExpr, BourneParameterExpansion, CaseItem, CommandSubstitutionSyntax, ConditionalExpr,
    ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Pattern, PatternPart, Position,
    PrefixMatchKind, Span, SubscriptSelector, VarRef, Word, WordPart, WordPartNode,
    ZshExpansionTarget,
};

use super::BacktickEscapedParameter;
use crate::Locator;
use shuck_semantic::{GlobPatternBehavior, PathnameExpansionBehavior, PatternOperatorBehavior};

mod arrays;
mod backticks;
mod command_substitution;
mod expansions;
mod globs;
mod shell_quoting;
mod source_scan;
mod zsh;

pub use arrays::*;
pub(crate) use backticks::{
    backtick_double_escaped_parameter_spans, backtick_escaped_parameter_reference_spans,
    backtick_escaped_parameters, backtick_substitution_spans,
    shellcheck_collapsed_backtick_part_span,
};
pub use command_substitution::*;
pub use expansions::*;
pub use globs::*;
pub use shell_quoting::*;
pub(crate) use source_scan::*;
pub use zsh::*;
