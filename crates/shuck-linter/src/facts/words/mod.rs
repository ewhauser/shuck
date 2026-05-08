use super::*;
use crate::Locator;

mod traversal;

#[allow(unused_imports)]
pub(in crate::facts) use traversal::*;

include!("expansion.rs");
include!("occurrence.rs");
include!("arithmetic.rs");
include!("command_facts.rs");
include!("quote_spans.rs");
include!("tests.rs");
