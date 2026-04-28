#[derive(Debug, Clone)]
pub struct FunctionHeaderFact<'a> {
    command_id: CommandId,
    function: &'a FunctionDef,
    binding_id: Option<BindingId>,
    scope_id: Option<ScopeId>,
    call_arity: FunctionCallArityFacts,
}

impl<'a> FunctionHeaderFact<'a> {
    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn function(&self) -> &'a FunctionDef {
        self.function
    }

    pub fn static_name_entry(&self) -> Option<(&'a Name, Span)> {
        self.function.static_name_entries().next()
    }

    pub fn binding_id(&self) -> Option<BindingId> {
        self.binding_id
    }

    pub fn function_scope(&self) -> Option<ScopeId> {
        self.scope_id
    }

    pub fn call_arity(&self) -> &FunctionCallArityFacts {
        &self.call_arity
    }

    pub fn function_span_in_source(&self, source: &str) -> Span {
        trim_trailing_whitespace_span(self.function.span, source)
    }

    pub fn span_in_source(&self, source: &str) -> Span {
        trim_trailing_whitespace_span(self.function.header.span(), source)
    }

    pub fn uses_function_keyword(&self) -> bool {
        self.function.uses_function_keyword()
    }

    pub fn has_trailing_parens(&self) -> bool {
        self.function.has_trailing_parens()
    }

    pub fn function_keyword_span(&self) -> Option<Span> {
        self.function.header.function_keyword_span
    }

    pub fn trailing_parens_span(&self) -> Option<Span> {
        self.function.header.trailing_parens_span
    }
}

#[derive(Debug, Clone, Default)]
pub struct FunctionCallArityFacts {
    call_count: usize,
    min_arg_count: usize,
    max_arg_count: usize,
    zero_arg_call_spans: Vec<Span>,
    zero_arg_diagnostic_spans: Vec<Span>,
}

impl FunctionCallArityFacts {
    pub fn call_count(&self) -> usize {
        self.call_count
    }

    pub fn min_arg_count(&self) -> Option<usize> {
        (self.call_count != 0).then_some(self.min_arg_count)
    }

    pub fn max_arg_count(&self) -> Option<usize> {
        (self.call_count != 0).then_some(self.max_arg_count)
    }

    pub fn called_only_without_args(&self) -> bool {
        self.call_count != 0 && self.max_arg_count == 0
    }

    pub fn zero_arg_call_spans(&self) -> &[Span] {
        &self.zero_arg_call_spans
    }

    pub fn zero_arg_diagnostic_spans(&self) -> &[Span] {
        &self.zero_arg_diagnostic_spans
    }

    fn record_call(&mut self, arg_count: usize, name_span: Span, diagnostic_span: Span) {
        if self.call_count == 0 {
            self.min_arg_count = arg_count;
            self.max_arg_count = arg_count;
        } else {
            self.min_arg_count = self.min_arg_count.min(arg_count);
            self.max_arg_count = self.max_arg_count.max(arg_count);
        }
        if arg_count == 0 {
            self.zero_arg_call_spans.push(name_span);
            self.zero_arg_diagnostic_spans.push(diagnostic_span);
        }
        self.call_count += 1;
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FunctionCliDispatchFacts {
    exported_from_case_cli: bool,
    dispatcher_span: Option<Span>,
}

impl FunctionCliDispatchFacts {
    pub fn exported_from_case_cli(self) -> bool {
        self.exported_from_case_cli
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn dispatcher_span(self) -> Option<Span> {
        self.dispatcher_span
    }

    fn record_dispatch(&mut self, span: Span) {
        self.exported_from_case_cli = true;
        self.dispatcher_span.get_or_insert(span);
    }
}

#[derive(Debug, Clone, Copy)]
struct FunctionFactInput<'a> {
    command_id: CommandId,
    function: &'a FunctionDef,
}

#[cfg_attr(shuck_profiling, inline(never))]
fn build_function_header_facts<'a>(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    functions: &[FunctionFactInput<'a>],
    commands: &[CommandFact<'a>],
    command_fact_indices_by_id: &[Option<usize>],
    source: &str,
) -> Vec<FunctionHeaderFact<'a>> {
    let call_arity_by_binding = build_function_call_arity_facts(
        semantic_analysis,
        &functions
            .iter()
            .map(|input| input.function)
            .collect::<Vec<_>>(),
        commands,
        command_fact_indices_by_id,
        source,
    );
    functions
        .iter()
        .copied()
        .map(|input| {
            let binding_id = semantic
                .function_definition_binding_for_command_span(semantic.command_span(input.command_id));
            let scope_id =
                binding_id.and_then(|binding_id| semantic_analysis.function_scope_for_binding(binding_id));
            let call_arity = binding_id
                .and_then(|binding_id| call_arity_by_binding.get(&binding_id).cloned())
                .unwrap_or_default();

            FunctionHeaderFact {
                command_id: input.command_id,
                function: input.function,
                binding_id,
                scope_id,
                call_arity,
            }
        })
        .collect()
}

#[cfg_attr(shuck_profiling, inline(never))]
fn build_function_cli_dispatch_facts(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    file: &File,
    source: &str,
) -> FxHashMap<ScopeId, FunctionCliDispatchFacts> {
    let mut facts = FxHashMap::<ScopeId, FunctionCliDispatchFacts>::default();

    for pair in file.body.as_slice().windows(2) {
        let [case_stmt, trailing_exit_stmt] = pair else {
            continue;
        };
        let Command::Compound(CompoundCommand::Case(case_command)) = &case_stmt.command else {
            continue;
        };
        if case_subject_variable_name(&case_command.word) != Some("1") {
            continue;
        }
        if !stmt_is_top_level_exit(trailing_exit_stmt) {
            continue;
        }

        for item in &case_command.cases {
            let Some(dispatcher_span) = first_positional_dispatch_in_commands(&item.body) else {
                continue;
            };

            for pattern in &item.patterns {
                let Some(name) = static_case_pattern_text(pattern, source) else {
                    continue;
                };
                if !is_plausible_shell_function_name(&name) {
                    continue;
                }

                let name = Name::from(name.as_str());
                let Some(binding_id) = semantic_analysis.visible_function_binding_defined_before(
                    &name,
                    semantic.scope_at(dispatcher_span.start.offset),
                    dispatcher_span.start.offset,
                ) else {
                    continue;
                };
                let Some(scope) = semantic_analysis.function_scope_for_binding(binding_id) else {
                    continue;
                };

                facts
                    .entry(scope)
                    .or_default()
                    .record_dispatch(dispatcher_span);
            }
        }
    }

    facts
}

#[cfg_attr(shuck_profiling, inline(never))]
fn build_case_cli_reachable_function_scopes(
    semantic: &SemanticModel,
    function_headers: &[FunctionHeaderFact<'_>],
    function_cli_dispatch_facts: &FxHashMap<ScopeId, FunctionCliDispatchFacts>,
    commands: &[CommandFact<'_>],
    command_parent_ids: &[Option<CommandId>],
) -> FxHashSet<ScopeId> {
    let dispatcher_offset = function_headers
        .iter()
        .filter_map(|header| {
            let scope = header.function_scope()?;
            function_cli_dispatch_facts
                .get(&scope)
                .copied()
                .unwrap_or_default()
                .dispatcher_span()
                .map(|span| span.start.offset)
        })
        .min();
    let top_level_exit_offset = commands
        .iter()
        .filter(|command| {
            command_parent_ids
                .get(command.id().index())
                .copied()
                .flatten()
                .is_none()
                && semantic
                    .ancestor_scopes(command.scope())
                    .all(|scope| {
                        !matches!(semantic.scope(scope).kind, shuck_semantic::ScopeKind::Function(_))
                    })
                && command_fact_is_standalone_exit(command)
        })
        .map(|command| command.span().start.offset)
        .min();

    function_headers
        .iter()
        .filter_map(|header| {
            let scope = header.function_scope()?;
            let nested = semantic
                .ancestor_scopes(scope)
                .skip(1)
                .any(|ancestor| {
                    matches!(semantic.scope(ancestor).kind, shuck_semantic::ScopeKind::Function(_))
                });
            (nested
                || dispatcher_offset
                    .is_some_and(|offset| header.function().span.start.offset < offset)
                || top_level_exit_offset
                    .is_some_and(|offset| header.function().span.start.offset < offset))
            .then_some(scope)
        })
        .collect()
}

fn stmt_is_top_level_exit(stmt: &Stmt) -> bool {
    if stmt.negated || matches!(stmt.terminator, Some(StmtTerminator::Background(_))) {
        return false;
    }

    let Command::Builtin(BuiltinCommand::Exit(command)) = &stmt.command else {
        return false;
    };
    if !command.extra_args.is_empty()
        || !command.assignments.is_empty()
        || !stmt.redirects.is_empty()
    {
        return false;
    }

    true
}

fn command_fact_is_standalone_exit(command: &CommandFact<'_>) -> bool {
    if command.stmt().negated
        || matches!(
            command.stmt().terminator,
            Some(StmtTerminator::Background(_))
        )
    {
        return false;
    }

    let Command::Builtin(BuiltinCommand::Exit(exit)) = command.command() else {
        return false;
    };
    exit.extra_args.is_empty() && exit.assignments.is_empty() && command.stmt().redirects.is_empty()
}

fn build_function_parameter_fallback_spans(
    commands: &[CommandFact<'_>],
    command_fact_indices_by_id: &[Option<usize>],
    structural_command_ids: &[CommandId],
    source: &str,
) -> Vec<Span> {
    let structural_commands = structural_command_ids
        .iter()
        .copied()
        .filter_map(|id| {
            command_fact_indices_by_id
                .get(id.index())
                .copied()
                .flatten()
                .and_then(|index| commands.get(index))
        })
        .collect::<Vec<_>>();

    structural_commands
        .windows(2)
        .filter_map(|pair| function_parameter_fallback_span(pair, source))
        .chain(
            commands
                .iter()
                .filter_map(named_coproc_subshell_fallback_span),
        )
        .collect()
}

fn build_completion_registered_function_command_flags(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    commands: &[CommandFact<'_>],
    command_fact_indices_by_id: &[Option<usize>],
    lists: &[ListFact<'_>],
    source: &str,
) -> Vec<bool> {
    let registered_scopes = build_completion_registered_function_scopes(
        semantic,
        commands,
        command_fact_indices_by_id,
        lists,
        source,
    );

    let mut flags = vec![false; function_command_slot_count(commands)];
    for command in commands {
        flags[command.id().index()] = semantic_analysis
            .enclosing_function_scope_at(command.span().start.offset)
            .is_some_and(|scope| registered_scopes.contains(&scope));
    }
    flags
}

fn build_completion_registered_function_scopes(
    semantic: &SemanticModel,
    commands: &[CommandFact<'_>],
    command_fact_indices_by_id: &[Option<usize>],
    lists: &[ListFact<'_>],
    source: &str,
) -> FxHashSet<ScopeId> {
    let mut function_candidates = Vec::new();
    function_candidates.resize_with(function_command_slot_count(commands), || None);
    for command in commands {
        function_candidates[command.id().index()] =
            completion_registered_function_candidate(semantic, command);
    }
    let mut scopes = FxHashSet::default();

    for list in lists {
        for (index, segment) in list.segments().iter().enumerate() {
            let Some(candidate) = function_candidates[segment.command_id().index()].as_ref() else {
                continue;
            };

            if list.segments()[index + 1..].iter().any(|later_segment| {
                command_registers_completion_function(
                    command_fact(
                        commands,
                        command_fact_indices_by_id,
                        later_segment.command_id(),
                    ),
                    source,
                    &candidate.name,
                )
            }) {
                scopes.insert(candidate.scope);
            }
        }
    }

    scopes
}

fn function_command_slot_count(commands: &[CommandFact<'_>]) -> usize {
    commands
        .iter()
        .map(|command| command.id().index())
        .max()
        .map_or(0, |index| index + 1)
}

fn completion_registered_function_candidate(
    semantic: &SemanticModel,
    command: &CommandFact<'_>,
) -> Option<CompletionRegisteredFunctionCandidate> {
    let Command::Function(function) = command.command() else {
        return None;
    };
    let (name, _) = function.static_name_entries().next()?;
    let scope = semantic.scope_at(function.body.span.start.offset);

    Some(CompletionRegisteredFunctionCandidate {
        scope,
        name: name.as_str().to_owned().into_boxed_str(),
    })
}

fn command_registers_completion_function(
    command: &CommandFact<'_>,
    source: &str,
    expected_name: &str,
) -> bool {
    if command.effective_or_literal_name() != Some("complete") {
        return false;
    }

    let mut expects_function_name = false;
    for word in command.body_args() {
        let Some(text) = static_word_text(word, source) else {
            expects_function_name = false;
            continue;
        };

        if expects_function_name {
            return text == expected_name;
        }

        if text == "--" {
            return false;
        }
        if text == "-F" || text == "--function" {
            expects_function_name = true;
            continue;
        }
        if let Some(name) = text.strip_prefix("-F")
            && !name.is_empty()
        {
            return name == expected_name;
        }
        if let Some(name) = text.strip_prefix("--function=") {
            return name == expected_name;
        }
    }

    false
}

#[derive(Debug, Clone)]
struct CompletionRegisteredFunctionCandidate {
    scope: ScopeId,
    name: Box<str>,
}

fn function_parameter_fallback_span(pair: &[&CommandFact<'_>], source: &str) -> Option<Span> {
    let [first, second] = pair else {
        return None;
    };
    let name = first.normalized().effective_or_literal_name()?;
    if !is_plausible_shell_function_name(name) || !first.normalized().body_args().is_empty() {
        return None;
    }
    if !matches!(first.command(), Command::Simple(_)) {
        return None;
    }
    let Command::Compound(CompoundCommand::Subshell(commands)) = second.command() else {
        return None;
    };
    if commands.is_empty() {
        return None;
    }
    if first.span().start.line != second.span().start.line {
        return None;
    }
    let tail = source.get(second.span().end.offset..)?;
    if !matches!(next_function_body_delimiter(tail), Some('{') | Some('(')) {
        return None;
    }
    let text = first.span().slice(source);
    let relative = text.find('(')?;
    let start = first.span().start.advanced_by(&text[..relative]);
    Some(Span::from_positions(start, start.advanced_by("(")))
}

fn named_coproc_subshell_fallback_span(command: &CommandFact<'_>) -> Option<Span> {
    let Command::Compound(CompoundCommand::Coproc(coproc)) = command.command() else {
        return None;
    };
    coproc.name_span?;
    let Command::Compound(CompoundCommand::Subshell(commands)) = &coproc.body.command else {
        return None;
    };
    if commands.is_empty() {
        return None;
    }
    let body_start = coproc.body.span.start;
    if coproc.span.start.line != body_start.line {
        return None;
    }
    Some(Span::from_positions(body_start, body_start))
}
fn build_function_call_arity_facts<'a>(
    semantic_analysis: &SemanticAnalysis<'_>,
    functions: &[&FunctionDef],
    commands: &[CommandFact<'a>],
    command_fact_indices_by_id: &[Option<usize>],
    source: &str,
) -> FxHashMap<BindingId, FunctionCallArityFacts> {
    let mut facts = FxHashMap::<BindingId, FunctionCallArityFacts>::default();
    let mut seen_names = FxHashSet::default();
    let mut unique_function_names: Vec<&Name> = Vec::with_capacity(functions.len());
    for function in functions {
        let Some((name, _)) = function.static_name_entries().next() else {
            continue;
        };
        if seen_names.insert(name.clone()) {
            unique_function_names.push(name);
        }
    }
    if unique_function_names.is_empty() {
        return facts;
    }

    let mut offsets = Vec::new();
    for name in &unique_function_names {
        for (site, _) in semantic_analysis.function_call_arity_sites(name) {
            offsets.push(site.name_span.start.offset);
        }
    }
    if offsets.is_empty() {
        return facts;
    }

    let command_ids_by_offset = build_innermost_command_ids_by_offset(commands, offsets);

    for name in unique_function_names {
        for (site, binding_id) in semantic_analysis.function_call_arity_sites(name) {
            let Some(command_id) =
                precomputed_command_id_for_offset(&command_ids_by_offset, site.name_span.start.offset)
            else {
                continue;
            };
            let command = command_fact(commands, command_fact_indices_by_id, command_id);
            if !command.wrappers().is_empty()
                || command.effective_or_literal_name() != Some(name.as_str())
            {
                continue;
            }
            let Some(name_word) = command.body_name_word() else {
                continue;
            };
            facts
                .entry(binding_id)
                .or_default()
                .record_call(
                    site.arg_count,
                    name_word.span,
                    function_call_diagnostic_span(command, name_word.span, source),
                );
        }
    }

    facts
}

fn function_call_diagnostic_span(
    command: &CommandFact<'_>,
    name_span: Span,
    source: &str,
) -> Span {
    if command.redirects().is_empty() {
        return name_span;
    }

    trim_trailing_whitespace_span(command.stmt().span, source)
}

fn first_positional_dispatch_in_commands(commands: &StmtSeq) -> Option<Span> {
    commands
        .iter()
        .find_map(|stmt| first_positional_dispatch_in_command(&stmt.command))
}

fn first_positional_dispatch_in_command(command: &Command) -> Option<Span> {
    match command {
        Command::Binary(command) => first_positional_dispatch_in_command(&command.left.command)
            .or_else(|| first_positional_dispatch_in_command(&command.right.command)),
        Command::Compound(CompoundCommand::BraceGroup(commands))
        | Command::Compound(CompoundCommand::Subshell(commands)) => {
            first_positional_dispatch_in_commands(commands)
        }
        Command::Compound(CompoundCommand::If(command)) => {
            first_positional_dispatch_in_commands(&command.condition)
                .or_else(|| first_positional_dispatch_in_commands(&command.then_branch))
                .or_else(|| {
                    command
                        .elif_branches
                        .iter()
                        .find_map(|(condition, branch)| {
                            first_positional_dispatch_in_commands(condition)
                                .or_else(|| first_positional_dispatch_in_commands(branch))
                        })
                })
                .or_else(|| {
                    command
                        .else_branch
                        .as_ref()
                        .and_then(first_positional_dispatch_in_commands)
                })
        }
        Command::Compound(CompoundCommand::While(command)) => {
            first_positional_dispatch_in_commands(&command.condition)
                .or_else(|| first_positional_dispatch_in_commands(&command.body))
        }
        Command::Compound(CompoundCommand::Until(command)) => {
            first_positional_dispatch_in_commands(&command.condition)
                .or_else(|| first_positional_dispatch_in_commands(&command.body))
        }
        Command::Compound(CompoundCommand::For(command)) => {
            first_positional_dispatch_in_commands(&command.body)
        }
        Command::Compound(CompoundCommand::Select(command)) => {
            first_positional_dispatch_in_commands(&command.body)
        }
        Command::Compound(CompoundCommand::Case(command)) => command
            .cases
            .iter()
            .find_map(|item| first_positional_dispatch_in_commands(&item.body)),
        Command::Compound(CompoundCommand::Time(command)) => command
            .command
            .as_ref()
            .and_then(|stmt| first_positional_dispatch_in_command(&stmt.command)),
        Command::Compound(CompoundCommand::Conditional(_))
        | Command::Compound(_)
        | Command::Builtin(_)
        | Command::Decl(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => None,
        Command::Simple(command) => {
            word_is_plain_positional_parameter(&command.name, "1").then_some(command.name.span)
        }
    }
}

fn word_is_plain_positional_parameter(word: &Word, target: &str) -> bool {
    let [part] = word.parts.as_slice() else {
        return false;
    };

    word_part_is_plain_positional_parameter(&part.kind, target)
}

fn word_part_is_plain_positional_parameter(part: &WordPart, target: &str) -> bool {
    match part {
        WordPart::Variable(name) => name.as_str() == target,
        WordPart::DoubleQuoted { parts, .. } => {
            let [part] = parts.as_slice() else {
                return false;
            };
            word_part_is_plain_positional_parameter(&part.kind, target)
        }
        WordPart::Parameter(parameter) => match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference }) => {
                reference.subscript.is_none() && reference.name.as_str() == target
            }
            ParameterExpansionSyntax::Zsh(syntax) => {
                syntax.length_prefix.is_none()
                    && syntax.operation.is_none()
                    && syntax.modifiers.is_empty()
                    && matches!(
                        &syntax.target,
                        ZshExpansionTarget::Reference(reference)
                            if reference.subscript.is_none()
                                && reference.name.as_str() == target
                    )
            }
            _ => false,
        },
        _ => false,
    }
}

fn function_body_without_braces_span(function: &FunctionDef) -> Option<Span> {
    match &function.body.command {
        Command::Compound(
            CompoundCommand::BraceGroup(_)
            | CompoundCommand::Subshell(_)
            | CompoundCommand::Arithmetic(_),
        ) => None,
        Command::Compound(_) => Some(function.body.span),
        Command::Simple(_)
        | Command::Decl(_)
        | Command::Builtin(_)
        | Command::Binary(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => None,
    }
}

fn next_function_body_delimiter(text: &str) -> Option<char> {
    let mut tail = text;

    loop {
        tail = trim_shell_layout_prefix(tail);

        if let Some(rest) = tail.strip_prefix('#') {
            tail = rest.split_once('\n').map_or("", |(_, rest)| rest);
            continue;
        }

        return tail.chars().next();
    }
}

fn trim_shell_layout_prefix(text: &str) -> &str {
    let mut tail = text;

    loop {
        tail = tail.trim_start_matches([' ', '\t', '\r', '\n']);

        if let Some(rest) = tail
            .strip_prefix("\\\r\n")
            .or_else(|| tail.strip_prefix("\\\n"))
        {
            tail = rest;
            continue;
        }

        return tail;
    }
}

fn is_plausible_shell_function_name(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    if !matches!(first, 'a'..='z' | 'A'..='Z' | '_') {
        return false;
    }
    if !name
        .chars()
        .all(|ch| matches!(ch, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-'))
    {
        return false;
    }
    !matches!(
        name,
        "!" | "{"
            | "}"
            | "if"
            | "then"
            | "else"
            | "elif"
            | "fi"
            | "do"
            | "done"
            | "case"
            | "esac"
            | "for"
            | "in"
            | "while"
            | "until"
            | "time"
            | "[["
            | "]]"
            | "function"
            | "select"
            | "coproc"
    )
}

fn build_function_positional_parameter_facts(
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    commands: &[CommandFact<'_>],
    positional_parameter_fragments: &[PositionalParameterFragmentFact],
) -> FxHashMap<ScopeId, FunctionPositionalParameterFacts> {
    let mut facts: FxHashMap<ScopeId, FunctionPositionalParameterFacts> = FxHashMap::default();
    let mut local_reset_offsets_by_scope: FxHashMap<ScopeId, Vec<usize>> = FxHashMap::default();

    for command in commands {
        if !command
            .options()
            .set()
            .is_some_and(|set| set.resets_positional_parameters())
        {
            continue;
        }

        let offset = command.span().start.offset;
        if let Some(scope) = innermost_nonpersistent_scope_within_function(semantic, command.scope())
        {
            local_reset_offsets_by_scope
                .entry(scope)
                .or_default()
                .push(offset);
        }
    }

    for reference in semantic.references() {
        if reference_has_local_positional_reset(
            semantic,
            reference.scope,
            reference.span.start.offset,
            &local_reset_offsets_by_scope,
        ) {
            continue;
        }

        let Some(index) = positional_parameter_index(reference.name.as_str()) else {
            let Some(uses_positional_parameters) =
                special_positional_parameter_name(reference.name.as_str())
            else {
                continue;
            };

            if semantic.is_guarded_parameter_reference(reference.id) {
                continue;
            }

            let Some(scope) =
                semantic_analysis.enclosing_function_scope_at(reference.span.start.offset)
            else {
                continue;
            };

            if uses_positional_parameters {
                facts
                    .entry(scope)
                    .or_default()
                    .uses_unprotected_positional_parameters = true;
            }
            continue;
        };
        if semantic.is_guarded_parameter_reference(reference.id) {
            continue;
        }

        let Some(scope) = semantic_analysis.enclosing_function_scope_at(reference.span.start.offset)
        else {
            continue;
        };

        let entry = facts.entry(scope).or_default();
        entry.required_arg_count = entry.required_arg_count.max(index);
        entry.uses_unprotected_positional_parameters = true;
    }

    for fragment in positional_parameter_fragments {
        if fragment.is_guarded() {
            continue;
        }

        let fragment_offset = fragment.span().start.offset;
        let fragment_scope = semantic.scope_at(fragment_offset);
        if reference_has_local_positional_reset(
            semantic,
            fragment_scope,
            fragment_offset,
            &local_reset_offsets_by_scope,
        ) {
            continue;
        }

        let Some(scope) = semantic_analysis.enclosing_function_scope_at(fragment_offset) else {
            continue;
        };

        facts
            .entry(scope)
            .or_default()
            .uses_unprotected_positional_parameters = true;
    }

    for command in commands {
        let Some(scope) =
            enclosing_function_scope_for_positional_reset(semantic, command.scope())
        else {
            continue;
        };

        if command
            .options()
            .set()
            .is_some_and(|set| set.resets_positional_parameters())
        {
            facts.entry(scope).or_default().resets_positional_parameters = true;
        }
    }

    facts
}

fn enclosing_function_scope_for_positional_reset(
    semantic: &SemanticModel,
    scope: ScopeId,
) -> Option<ScopeId> {
    for scope in semantic.ancestor_scopes(scope) {
        match semantic.scope_kind(scope) {
            shuck_semantic::ScopeKind::Function(_) => return Some(scope),
            shuck_semantic::ScopeKind::Subshell
            | shuck_semantic::ScopeKind::CommandSubstitution
            | shuck_semantic::ScopeKind::Pipeline => return None,
            shuck_semantic::ScopeKind::File => {}
        }
    }

    None
}

fn innermost_nonpersistent_scope_within_function(
    semantic: &SemanticModel,
    scope: ScopeId,
) -> Option<ScopeId> {
    for scope in semantic.ancestor_scopes(scope) {
        match semantic.scope_kind(scope) {
            shuck_semantic::ScopeKind::Subshell
            | shuck_semantic::ScopeKind::CommandSubstitution
            | shuck_semantic::ScopeKind::Pipeline => return Some(scope),
            shuck_semantic::ScopeKind::Function(_) => return None,
            shuck_semantic::ScopeKind::File => {}
        }
    }

    None
}

fn reference_has_local_positional_reset(
    semantic: &SemanticModel,
    scope: ScopeId,
    offset: usize,
    local_reset_offsets_by_scope: &FxHashMap<ScopeId, Vec<usize>>,
) -> bool {
    for scope in semantic.ancestor_scopes(scope) {
        match semantic.scope_kind(scope) {
            shuck_semantic::ScopeKind::Subshell
            | shuck_semantic::ScopeKind::CommandSubstitution
            | shuck_semantic::ScopeKind::Pipeline => {
                if local_reset_offsets_by_scope
                    .get(&scope)
                    .is_some_and(|offsets| {
                        offsets.iter().any(|reset_offset| *reset_offset < offset)
                    })
                {
                    return true;
                }
            }
            shuck_semantic::ScopeKind::Function(_) => return false,
            shuck_semantic::ScopeKind::File => {}
        }
    }

    false
}

fn positional_parameter_index(name: &str) -> Option<usize> {
    if name == "0" || matches!(name, "@" | "*" | "#") {
        return None;
    }
    if name.chars().all(|ch| ch.is_ascii_digit()) {
        name.parse::<usize>().ok()
    } else {
        None
    }
}

fn special_positional_parameter_name(name: &str) -> Option<bool> {
    match name {
        "@" | "*" | "#" => Some(true),
        _ => None,
    }
}
