use shuck_ast::{
    ArithmeticExpr, BourneParameterExpansion, CaseItem, CommandSubstitutionSyntax, ConditionalExpr,
    ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Pattern, PatternGroupKind,
    PatternPart, Position, PrefixMatchKind, Span, SubscriptSelector, VarRef, Word, WordPart,
    WordPartNode, ZshExpansionTarget,
};

use super::BacktickEscapedParameter;

mod arrays;
mod backticks;
mod command_substitution;
mod expansions;
mod globs;
mod shell_quoting;
mod source_scan;
mod zsh;

pub use arrays::*;
pub use backticks::*;
pub use command_substitution::*;
pub use expansions::*;
pub use globs::*;
pub use shell_quoting::*;
pub use source_scan::*;
pub use zsh::*;

#[allow(unused_imports)]
use {
    arrays::*, backticks::*, command_substitution::*, expansions::*, globs::*, shell_quoting::*,
    source_scan::*, zsh::*,
};
