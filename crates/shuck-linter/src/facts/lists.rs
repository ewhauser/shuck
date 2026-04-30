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
pub struct ListSegmentFact<'a> {
    command_id: CommandId,
    span: Span,
    kind: ListSegmentKind,
    assignment_target: Option<&'a str>,
    assignment_span: Option<Span>,
    assignment_is_declaration: bool,
}

impl<'a> ListSegmentFact<'a> {
    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn kind(&self) -> ListSegmentKind {
        self.kind
    }

    pub fn assignment_target(&self) -> Option<&'a str> {
        self.assignment_target
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
pub struct ListFact<'a> {
    key: FactSpan,
    command: &'a BinaryCommand,
    operators: Box<[ListOperatorFact]>,
    segments: Box<[ListSegmentFact<'a>]>,
    mixed_short_circuit_span: Option<Span>,
    mixed_short_circuit_kind: Option<MixedShortCircuitKind>,
}

impl<'a> ListFact<'a> {
    pub fn key(&self) -> FactSpan {
        self.key
    }

    pub fn command(&self) -> &'a BinaryCommand {
        self.command
    }

    pub fn span(&self) -> Span {
        self.command.span
    }

    pub fn operators(&self) -> &[ListOperatorFact] {
        &self.operators
    }

    pub fn segments(&self) -> &[ListSegmentFact<'a>] {
        &self.segments
    }

    pub fn mixed_short_circuit_span(&self) -> Option<Span> {
        self.mixed_short_circuit_span
    }

    pub fn mixed_short_circuit_kind(&self) -> Option<MixedShortCircuitKind> {
        self.mixed_short_circuit_kind
    }
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(super) fn build_list_facts<'a>(
    commands: &[CommandFact<'a>],
    command_fact_indices_by_id: &[Option<usize>],
    command_ids_by_span: &CommandLookupIndex,
    command_child_index: &CommandChildIndex,
    source: &str,
) -> Vec<ListFact<'a>> {
    let command_relationships = CommandRelationshipContext::new(
        commands,
        command_fact_indices_by_id,
        command_ids_by_span,
        command_child_index,
    );
    let mut nested_list_commands = FxHashSet::default();

    for fact in commands {
        let Command::Binary(command) = fact.command() else {
            continue;
        };
        if !matches!(command.op, BinaryOp::And | BinaryOp::Or) {
            continue;
        }

        record_nested_list_command(
            &command.left,
            fact.id(),
            command_relationships,
            &mut nested_list_commands,
        );
        record_nested_list_command(
            &command.right,
            fact.id(),
            command_relationships,
            &mut nested_list_commands,
        );
    }

    commands
        .iter()
        .filter_map(|fact| {
            let Command::Binary(command) = fact.command() else {
                return None;
            };
            if !matches!(command.op, BinaryOp::And | BinaryOp::Or)
                || nested_list_commands.contains(&fact.id())
            {
                return None;
            }

            let mut operators = Vec::new();
            collect_short_circuit_operators(command, &mut operators);
            let segments = build_list_segment_facts(
                command,
                command_relationships,
                fact.id(),
                source,
            )?;
            let mixed_short_circuit_span = mixed_short_circuit_operator_span(&operators);
            let mixed_short_circuit_kind = mixed_short_circuit_span
                .map(|_| classify_mixed_short_circuit_kind(&segments, &operators));

            Some(ListFact {
                key: fact.key(),
                command,
                operators: operators.into_boxed_slice(),
                segments,
                mixed_short_circuit_span,
                mixed_short_circuit_kind,
            })
        })
        .collect()
}

fn record_nested_list_command(
    stmt: &Stmt,
    parent_id: CommandId,
    command_relationships: CommandRelationshipContext<'_, '_>,
    nested_list_commands: &mut FxHashSet<CommandId>,
) {
    if matches!(
        &stmt.command,
        Command::Binary(child) if matches!(child.op, BinaryOp::And | BinaryOp::Or)
    ) && let Some(child) = command_relationships.child_or_lookup_fact(parent_id, stmt)
    {
        nested_list_commands.insert(child.id());
    }
}

fn build_list_segment_facts<'a>(
    command: &BinaryCommand,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    parent_id: CommandId,
    source: &str,
) -> Option<Box<[ListSegmentFact<'a>]>> {
    let mut segments = Vec::new();
    collect_list_segment_facts(
        command,
        command_relationships,
        parent_id,
        source,
        &mut segments,
    )?;
    Some(segments.into_boxed_slice())
}

fn collect_list_segment_facts<'a>(
    command: &BinaryCommand,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    parent_id: CommandId,
    source: &str,
    segments: &mut Vec<ListSegmentFact<'a>>,
) -> Option<()> {
    let mut stack = vec![(&command.right, parent_id), (&command.left, parent_id)];
    while let Some((stmt, parent_id)) = stack.pop() {
        if let Command::Binary(binary) = &stmt.command
            && matches!(binary.op, BinaryOp::And | BinaryOp::Or)
        {
            let nested_parent_id = command_relationships
                .child_or_lookup_fact(parent_id, stmt)?
                .id();
            stack.push((&binary.right, nested_parent_id));
            stack.push((&binary.left, nested_parent_id));
            continue;
        }

        push_list_stmt_segment_fact(stmt, command_relationships, parent_id, source, segments)?;
    }
    Some(())
}

fn push_list_stmt_segment_fact<'a>(
    stmt: &Stmt,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    parent_id: CommandId,
    source: &str,
    segments: &mut Vec<ListSegmentFact<'a>>,
) -> Option<()> {
    let fact = command_relationships.child_or_lookup_fact(parent_id, stmt)?;
    let id = fact.id();
    let assignment_info = list_segment_assignment_info(fact);
    let assignment_target = assignment_info.as_ref().map(|info| info.target);
    let assignment_is_declaration = assignment_info
        .as_ref()
        .is_some_and(|info| info.is_declaration);

    segments.push(ListSegmentFact {
        command_id: id,
        span: fact.span_in_source(source),
        kind: list_segment_kind(fact),
        assignment_target,
        assignment_span: assignment_info.map(|info| info.span),
        assignment_is_declaration,
    });
    Some(())
}

fn list_segment_kind(fact: &CommandFact<'_>) -> ListSegmentKind {
    if list_segment_is_condition(fact) {
        ListSegmentKind::Condition
    } else if list_segment_assignment_target(fact).is_some() {
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

fn list_segment_assignment_target<'a>(fact: &CommandFact<'a>) -> Option<&'a str> {
    list_segment_assignment_info(fact).map(|info| info.target)
}

#[derive(Clone, Copy)]
struct ListSegmentAssignmentInfo<'a> {
    target: &'a str,
    span: Span,
    is_declaration: bool,
}

fn list_segment_assignment_info<'a>(
    fact: &CommandFact<'a>,
) -> Option<ListSegmentAssignmentInfo<'a>> {
    match fact.command() {
        Command::Simple(command)
            if command.args.is_empty()
                && !command.assignments.is_empty()
                && fact.literal_name() == Some("") =>
        {
            single_assignment_info(&command.assignments)
        }
        Command::Decl(command) => declaration_assignment_info(command),
        _ => None,
    }
}

fn single_assignment_info<'a>(
    assignments: &'a [Assignment],
) -> Option<ListSegmentAssignmentInfo<'a>> {
    (assignments.len() == 1).then(|| ListSegmentAssignmentInfo {
        target: assignments[0].target.name.as_str(),
        span: assignments[0].span,
        is_declaration: false,
    })
}

fn declaration_assignment_info<'a>(
    command: &'a DeclClause,
) -> Option<ListSegmentAssignmentInfo<'a>> {
    if !command.assignments.is_empty() {
        return None;
    }

    let mut assignment = None;

    for operand in &command.operands {
        match operand {
            DeclOperand::Flag(_) => {}
            DeclOperand::Assignment(candidate) => {
                if assignment.replace(candidate).is_some() {
                    return None;
                }
            }
            DeclOperand::Name(_) | DeclOperand::Dynamic(_) => return None,
        }
    }

    assignment.map(|assignment| ListSegmentAssignmentInfo {
        target: assignment.target.name.as_str(),
        span: assignment.span,
        is_declaration: true,
    })
}

fn classify_mixed_short_circuit_kind(
    segments: &[ListSegmentFact<'_>],
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
    segments: &[ListSegmentFact<'_>],
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
