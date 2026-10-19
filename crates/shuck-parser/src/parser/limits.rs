//! Resource limits shared by one parse and every nested parser it starts.
//!
//! Command substitutions, subscripts, arithmetic operands, and similar
//! constructs are parsed by short-lived child parsers. All of them draw from a
//! single [`ParseBudget`] so a deeply nested input cannot reset its operation
//! count or its native-stack allowance at each nesting level.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use crate::error::Error;

/// Native stack the parser may consume, measured from the outermost parse
/// entry point.
///
/// Depth limits bound how deeply the AST nests, but the native stack cost of a
/// nesting level differs a lot between constructs and build profiles (debug
/// frames are several times larger than release frames). This budget is the
/// backstop that turns runaway recursion into a parse error instead of a stack
/// overflow. It leaves roughly 512 KiB of a 2 MiB thread stack (the Rust
/// default for spawned threads) for the caller and for the stack used between
/// two budget checks.
const PARSER_STACK_BUDGET: usize = 1536 * 1024;

/// Most operator-chain links (`&&`/`||`, `|`, `[[ ]]` operators, arithmetic
/// binary operators and commas) that may be open at once across all enclosing
/// chains.
///
/// Chains are parsed iteratively, but each link adds one level of left-nested
/// AST, and AST consumers (including `Drop`) walk them recursively. Counting
/// the links of every enclosing chain bounds that depth even when chains are
/// nested inside each other's operands.
pub(super) const MAX_OPERATOR_CHAIN_LEN: usize = 1000;

/// Operation and stack accounting shared across a parse tree.
#[derive(Debug)]
pub(crate) struct ParseBudget {
    fuel: AtomicUsize,
    max_fuel: usize,
    /// Depth limit configured for the outermost parser. Nested parsers may
    /// run under tighter caps; hitting one of those is a silent fallback, not
    /// a reportable error.
    max_depth: usize,
    /// Approximate stack address of the outermost parse entry, or `0` when
    /// no entry point has been recorded yet.
    stack_anchor: AtomicUsize,
    /// Operator-chain links currently open across the parse tree.
    chain_links: AtomicUsize,
    /// Most links open at once since the innermost pending first operand
    /// started; see [`ParseBudget::begin_first_chain_operand`].
    chain_peak: AtomicUsize,
    /// First resource-limit error hit anywhere in the parse tree.
    ///
    /// Nested parsers often discard their own errors and fall back to a
    /// literal interpretation, so limit errors are recorded here and reported
    /// once by the outermost parser.
    limit_error: Mutex<Option<Error>>,
}

impl ParseBudget {
    pub(super) fn new(max_fuel: usize, max_depth: usize) -> Arc<Self> {
        Arc::new(Self {
            fuel: AtomicUsize::new(max_fuel),
            max_fuel,
            max_depth,
            stack_anchor: AtomicUsize::new(0),
            chain_links: AtomicUsize::new(0),
            chain_peak: AtomicUsize::new(0),
            limit_error: Mutex::new(None),
        })
    }

    #[cfg(feature = "benchmarking")]
    pub(super) fn max_fuel(&self) -> usize {
        self.max_fuel
    }

    /// Consume one unit of fuel.
    pub(super) fn tick(&self) -> Result<(), Error> {
        // A parse tree is driven by a single thread, so a relaxed load/store
        // pair is enough; the atomic only exists so parsers stay `Send`.
        let fuel = self.fuel.load(Ordering::Relaxed);
        if fuel == 0 {
            return Err(self.record(Error::parse(format!(
                "parser fuel exhausted ({} operations, max {})",
                self.max_fuel, self.max_fuel
            ))));
        }
        self.fuel.store(fuel - 1, Ordering::Relaxed);
        Ok(())
    }

    /// Remember the current stack position as the parse entry point if none
    /// has been recorded yet.
    #[inline(never)]
    pub(super) fn anchor_stack(&self) {
        let here = current_stack_address();
        let _ = self
            .stack_anchor
            .compare_exchange(0, here, Ordering::Relaxed, Ordering::Relaxed);
    }

    /// Fail once the parse has used more than its native stack allowance.
    #[inline(never)]
    pub(super) fn check_stack(&self) -> Result<(), Error> {
        let here = current_stack_address();
        let anchor = self.stack_anchor.load(Ordering::Relaxed);
        if anchor == 0 {
            self.anchor_stack();
            return Ok(());
        }
        // Stacks grow downward on every supported target. On a target where
        // they grow upward this never fires and the depth limits still apply.
        if anchor.saturating_sub(here) > PARSER_STACK_BUDGET {
            return Err(self.record(Error::parse(
                "AST nesting too deep (parser stack budget exhausted)",
            )));
        }
        Ok(())
    }

    /// Record a resource-limit error so the outermost parser can report it,
    /// and return it for the caller to propagate.
    pub(super) fn record(&self, error: Error) -> Error {
        if let Ok(mut slot) = self.limit_error.lock()
            && slot.is_none()
        {
            *slot = Some(error.clone());
        }
        error
    }

    /// Error for nesting to `depth` past `max_depth`, recorded when it exceeds
    /// the configured limit rather than a tighter nested cap.
    pub(super) fn depth_error(&self, depth: usize, max_depth: usize) -> Error {
        let error = Error::parse(format!(
            "AST nesting too deep ({depth} levels, max {max_depth})"
        ));
        if depth > self.max_depth {
            self.record(error)
        } else {
            error
        }
    }

    /// Start measuring how deeply chains nest inside the first operand of a
    /// chain that may follow it.
    ///
    /// The first operand is parsed before its chain opens any links, so its
    /// own chains would otherwise go uncounted even though the AST nests them
    /// below the enclosing chain's left spine.
    pub(super) fn begin_first_chain_operand(&self) -> FirstChainOperand {
        let open = self.chain_links.load(Ordering::Relaxed);
        let saved_peak = self.chain_peak.swap(open, Ordering::Relaxed);
        FirstChainOperand { open, saved_peak }
    }

    /// Finish a first operand and start the chain that follows it, carrying
    /// the chain depth reached inside the operand.
    pub(super) fn end_first_chain_operand(&self, operand: FirstChainOperand) -> ChainLinks {
        let peak = self.chain_peak.load(Ordering::Relaxed);
        self.chain_peak
            .store(peak.max(operand.saved_peak), Ordering::Relaxed);
        ChainLinks {
            inherited: peak.saturating_sub(operand.open),
            chain: None,
        }
    }

    pub(super) fn take_limit_error(&self) -> Option<Error> {
        self.limit_error
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }
}

/// Marker returned by [`ParseBudget::begin_first_chain_operand`].
#[must_use]
pub(super) struct FirstChainOperand {
    open: usize,
    saved_peak: usize,
}

/// Links of one operator chain, counted against [`MAX_OPERATOR_CHAIN_LEN`]
/// together with every chain that encloses it and the chains nested in its
/// first operand. Dropping it releases its links.
pub(super) struct ChainLinks {
    inherited: usize,
    chain: Option<OperatorChain>,
}

struct OperatorChain {
    budget: Arc<ParseBudget>,
    links: usize,
}

impl ChainLinks {
    /// Add one link, opening the chain on the first call. Fails with the total
    /// open link count once it exceeds the limit.
    pub(super) fn extend(&mut self, budget: &Arc<ParseBudget>) -> Result<(), usize> {
        let added = if self.chain.is_some() {
            1
        } else {
            self.inherited + 1
        };
        let chain = self.chain.get_or_insert_with(|| OperatorChain {
            budget: Arc::clone(budget),
            links: 0,
        });
        chain.links += added;
        let total = chain.budget.chain_links.fetch_add(added, Ordering::Relaxed) + added;
        chain.budget.chain_peak.fetch_max(total, Ordering::Relaxed);
        if total > MAX_OPERATOR_CHAIN_LEN {
            return Err(total);
        }
        Ok(())
    }
}

impl Drop for OperatorChain {
    fn drop(&mut self) {
        self.budget
            .chain_links
            .fetch_sub(self.links, Ordering::Relaxed);
    }
}

/// Depth and budget handed to a nested parser.
#[derive(Debug, Clone)]
pub(super) struct NestedLimits {
    /// Absolute nesting depth the nested parser starts at.
    pub(super) base_depth: usize,
    /// Absolute nesting depth the nested parser may not exceed.
    pub(super) max_depth: usize,
    pub(super) budget: Arc<ParseBudget>,
}

impl NestedLimits {
    /// Limits for a parser that is not nested inside another parse.
    pub(super) fn root(max_depth: usize, max_fuel: usize) -> Self {
        Self {
            base_depth: 0,
            max_depth,
            budget: ParseBudget::new(max_fuel, max_depth),
        }
    }

    /// Limits for one more nesting level below `depth`. Fails when that level
    /// would exceed `max_depth` or the parse is out of stack.
    pub(super) fn below(
        depth: usize,
        max_depth: usize,
        budget: &Arc<ParseBudget>,
    ) -> Result<Self, Error> {
        budget.check_stack()?;
        let base_depth = depth + 1;
        if base_depth > max_depth {
            return Err(budget.depth_error(base_depth, max_depth));
        }
        Ok(Self {
            base_depth,
            max_depth,
            budget: Arc::clone(budget),
        })
    }

    /// Cap how many further levels the nested parser may add.
    pub(super) fn with_extra_depth_cap(mut self, levels: usize) -> Self {
        self.max_depth = self.max_depth.min(self.base_depth.saturating_add(levels));
        self
    }
}

#[inline(always)]
fn current_stack_address() -> usize {
    let marker = 0u8;
    std::hint::black_box(&marker) as *const u8 as usize
}
