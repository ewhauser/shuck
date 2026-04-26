#[derive(Debug, Clone, Copy)]
pub struct ListOperatorFact {
    op: BinaryOp,
    span: Span,
}

impl ListOperatorFact {
    pub fn op(&self) -> BinaryOp {
        self.op
    }

    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListSegmentKind {
    Condition,
    AssignmentOnly,
    Other,
}

#[derive(Debug, Clone)]
pub struct ListSegmentFact {
    command_id: CommandId,
    span: Span,
    kind: ListSegmentKind,
    assignment_target: Option<Box<str>>,
    assignment_span: Option<Span>,
    assignment_is_declaration: bool,
}

impl ListSegmentFact {
    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn kind(&self) -> ListSegmentKind {
        self.kind
    }

    pub fn assignment_target(&self) -> Option<&str> {
        self.assignment_target.as_deref()
    }

    pub fn assignment_span(&self) -> Option<Span> {
        self.assignment_span
    }

    pub fn assignment_is_declaration(&self) -> bool {
        self.assignment_is_declaration
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixedShortCircuitKind {
    TestChain,
    AssignmentTernary,
    Fallthrough,
}


#[derive(Debug, Clone)]
pub struct ListFact {
    key: FactSpan,
    arena_command_id: Option<AstCommandId>,
    span: Span,
    operators: Box<[ListOperatorFact]>,
    segments: Box<[ListSegmentFact]>,
    mixed_short_circuit_span: Option<Span>,
    mixed_short_circuit_kind: Option<MixedShortCircuitKind>,
}

impl ListFact {
    pub fn key(&self) -> FactSpan {
        self.key
    }

    pub fn arena_command_id(&self) -> Option<AstCommandId> {
        self.arena_command_id
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn operators(&self) -> &[ListOperatorFact] {
        &self.operators
    }

    pub fn segments(&self) -> &[ListSegmentFact] {
        &self.segments
    }

    pub fn mixed_short_circuit_span(&self) -> Option<Span> {
        self.mixed_short_circuit_span
    }

    pub fn mixed_short_circuit_kind(&self) -> Option<MixedShortCircuitKind> {
        self.mixed_short_circuit_kind
    }
}

pub(super) fn build_list_facts<'a>(
    commands: &[CommandFact<'a>],
    _command_ids_by_span: &CommandLookupIndex,
    command_child_index: &CommandChildIndex,
    arena_file: &ArenaFile,
    source: &str,
) -> Vec<ListFact> {
    let command_relationships = CommandRelationshipContext::new(commands, command_child_index);
    let mut nested_list_commands = FxHashSet::default();

    for fact in commands {
        let Some(command) = fact
            .arena_command_id()
            .and_then(|id| arena_file.store.command(id).binary())
        else {
            continue;
        };
        if !matches!(command.op(), BinaryOp::And | BinaryOp::Or) {
            continue;
        }

        record_nested_list_command(
            command.left(),
            fact.id(),
            command_relationships,
            &mut nested_list_commands,
        );
        record_nested_list_command(
            command.right(),
            fact.id(),
            command_relationships,
            &mut nested_list_commands,
        );
    }

    commands
        .iter()
        .filter_map(|fact| {
            let command = fact
                .arena_command_id()
                .and_then(|id| arena_file.store.command(id).binary())?;
            if !matches!(command.op(), BinaryOp::And | BinaryOp::Or)
                || nested_list_commands.contains(&fact.id())
            {
                return None;
            };

            let mut operators = Vec::new();
            collect_arena_short_circuit_operators(command, &mut operators);
            let segments = build_list_segment_facts(
                command,
                command_relationships,
                fact.id(),
                arena_file,
                source,
            )?;
            let mixed_short_circuit_span = mixed_short_circuit_operator_span(&operators);
            let mixed_short_circuit_kind = mixed_short_circuit_span
                .map(|_| classify_mixed_short_circuit_kind(&segments, &operators));

            Some(ListFact {
                key: fact.key(),
                arena_command_id: fact.arena_command_id(),
                span: fact.span(),
                operators: operators.into_boxed_slice(),
                segments,
                mixed_short_circuit_span,
                mixed_short_circuit_kind,
            })
        })
        .collect()
}

fn record_nested_list_command(
    seq: StmtSeqView<'_>,
    parent_id: CommandId,
    command_relationships: CommandRelationshipContext<'_, '_>,
    nested_list_commands: &mut FxHashSet<CommandId>,
) {
    for stmt in seq.stmts() {
        if stmt
            .command()
            .binary()
            .is_some_and(|child| matches!(child.op(), BinaryOp::And | BinaryOp::Or))
            && let Some(child) = command_relationships.child_or_lookup_arena_fact(parent_id, stmt)
        {
            nested_list_commands.insert(child.id());
        }
    }
}

fn build_list_segment_facts<'a>(
    command: BinaryCommandView<'_>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    parent_id: CommandId,
    arena_file: &ArenaFile,
    source: &str,
) -> Option<Box<[ListSegmentFact]>> {
    let mut segments = Vec::new();
    collect_list_segment_facts(
        command,
        command_relationships,
        parent_id,
        arena_file,
        source,
        &mut segments,
    )?;
    Some(segments.into_boxed_slice())
}

fn collect_list_segment_facts<'a>(
    command: BinaryCommandView<'_>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    parent_id: CommandId,
    arena_file: &ArenaFile,
    source: &str,
    segments: &mut Vec<ListSegmentFact>,
) -> Option<()> {
    collect_list_stmt_segment_facts(
        command.left(),
        command_relationships,
        parent_id,
        arena_file,
        source,
        segments,
    )?;
    collect_list_stmt_segment_facts(
        command.right(),
        command_relationships,
        parent_id,
        arena_file,
        source,
        segments,
    )?;
    Some(())
}

fn collect_list_stmt_segment_facts<'a>(
    seq: StmtSeqView<'_>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    parent_id: CommandId,
    arena_file: &ArenaFile,
    source: &str,
    segments: &mut Vec<ListSegmentFact>,
) -> Option<()> {
    for stmt in seq.stmts() {
        if let Some(binary) = stmt.command().binary()
            && matches!(binary.op(), BinaryOp::And | BinaryOp::Or)
        {
            let nested_parent_id = command_relationships
                .child_or_lookup_arena_fact(parent_id, stmt)?
                .id();
            collect_list_segment_facts(
                binary,
                command_relationships,
                nested_parent_id,
                arena_file,
                source,
                segments,
            )?;
            continue;
        }

        let fact = command_relationships.child_or_lookup_arena_fact(parent_id, stmt)?;
        let id = fact.id();
        let assignment_info = list_segment_assignment_info(fact, arena_file);
        let assignment_target = assignment_info
            .as_ref()
            .map(|info| info.target)
            .map(str::to_owned)
            .map(String::into_boxed_str);
        let assignment_is_declaration = assignment_info
            .as_ref()
            .is_some_and(|info| info.is_declaration);

        segments.push(ListSegmentFact {
            command_id: id,
            span: fact.span_in_source(source),
            kind: list_segment_kind(fact, arena_file),
            assignment_target,
            assignment_span: assignment_info.map(|info| info.span),
            assignment_is_declaration,
        });
    }
    Some(())
}

fn list_segment_kind(fact: &CommandFact<'_>, arena_file: &ArenaFile) -> ListSegmentKind {
    if list_segment_is_condition(fact) {
        ListSegmentKind::Condition
    } else if list_segment_assignment_info(fact, arena_file).is_some() {
        ListSegmentKind::AssignmentOnly
    } else {
        ListSegmentKind::Other
    }
}

fn list_segment_is_condition(fact: &CommandFact<'_>) -> bool {
    fact.simple_test().is_some()
        || fact.conditional().is_some()
        || matches!(fact.effective_or_literal_name(), Some("true" | "false"))
}

#[derive(Clone, Copy)]
struct ListSegmentAssignmentInfo<'a> {
    target: &'a str,
    span: Span,
    is_declaration: bool,
}

fn list_segment_assignment_info<'a>(
    fact: &'a CommandFact<'a>,
    arena_file: &'a ArenaFile,
) -> Option<ListSegmentAssignmentInfo<'a>> {
    let command = arena_file.store.command(fact.arena_command_id()?);
    match command.kind() {
        ArenaFileCommandKind::Simple
            if command.simple().is_some_and(|command| {
                command.arg_ids().is_empty() && !command.assignments().is_empty()
            }) && fact.literal_name() == Some("") =>
        {
            single_assignment_info(command.simple()?.assignments())
        }
        ArenaFileCommandKind::Decl => fact.single_declaration_assignment_info().map(|(target, span)| {
            ListSegmentAssignmentInfo {
                target,
                span,
                is_declaration: true,
            }
        }),
        _ => None,
    }
}

fn single_assignment_info<'a>(
    assignments: &'a [AssignmentNode],
) -> Option<ListSegmentAssignmentInfo<'a>> {
    (assignments.len() == 1).then(|| ListSegmentAssignmentInfo {
        target: assignments[0].target.name.as_str(),
        span: assignments[0].span,
        is_declaration: false,
    })
}

fn collect_arena_short_circuit_operators(
    command: BinaryCommandView<'_>,
    operators: &mut Vec<ListOperatorFact>,
) {
    if let Some(left) = single_binary_stmt(command.left())
        && matches!(left.op(), BinaryOp::And | BinaryOp::Or)
    {
        collect_arena_short_circuit_operators(left, operators);
    }

    if matches!(command.op(), BinaryOp::And | BinaryOp::Or) {
        operators.push(ListOperatorFact {
            op: command.op(),
            span: command.op_span(),
        });
    }

    if let Some(right) = single_binary_stmt(command.right())
        && matches!(right.op(), BinaryOp::And | BinaryOp::Or)
    {
        collect_arena_short_circuit_operators(right, operators);
    }
}

fn single_binary_stmt(seq: StmtSeqView<'_>) -> Option<BinaryCommandView<'_>> {
    let mut stmts = seq.stmts();
    let stmt = stmts.next()?;
    if stmts.next().is_some() {
        return None;
    }
    stmt.command().binary()
}

fn classify_mixed_short_circuit_kind(
    segments: &[ListSegmentFact],
    operators: &[ListOperatorFact],
) -> MixedShortCircuitKind {
    if segments
        .iter()
        .all(|segment| segment.kind() == ListSegmentKind::Condition)
    {
        MixedShortCircuitKind::TestChain
    } else if matches_assignment_ternary(segments, operators) {
        MixedShortCircuitKind::AssignmentTernary
    } else {
        MixedShortCircuitKind::Fallthrough
    }
}

fn matches_assignment_ternary(
    segments: &[ListSegmentFact],
    operators: &[ListOperatorFact],
) -> bool {
    let [condition, then_branch, else_branch] = segments else {
        return false;
    };
    let [first_operator, second_operator] = operators else {
        return false;
    };

    condition.kind() == ListSegmentKind::Condition
        && first_operator.op() == BinaryOp::And
        && second_operator.op() == BinaryOp::Or
        && then_branch.kind() == ListSegmentKind::AssignmentOnly
        && else_branch.kind() == ListSegmentKind::AssignmentOnly
        && then_branch.assignment_target().is_some()
        && then_branch.assignment_target() == else_branch.assignment_target()
}
