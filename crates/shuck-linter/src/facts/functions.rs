#[derive(Debug, Clone)]
pub struct FunctionHeaderFact<'a> {
    function: &'a FunctionDef,
    binding_id: Option<BindingId>,
    scope_id: Option<ScopeId>,
    call_arity: FunctionCallArityFacts,
}

impl<'a> FunctionHeaderFact<'a> {
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

    fn record_call(&mut self, arg_count: usize, span: Span) {
        if self.call_count == 0 {
            self.min_arg_count = arg_count;
            self.max_arg_count = arg_count;
        } else {
            self.min_arg_count = self.min_arg_count.min(arg_count);
            self.max_arg_count = self.max_arg_count.max(arg_count);
        }
        if arg_count == 0 {
            self.zero_arg_call_spans.push(span);
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

fn build_function_header_facts<'a>(
    semantic: &SemanticModel,
    functions: &[&'a FunctionDef],
    commands: &[CommandFact<'a>],
    source: &str,
) -> Vec<FunctionHeaderFact<'a>> {
    let call_arity_by_binding =
        build_function_call_arity_facts(semantic, functions, commands, source);
    functions
        .iter()
        .copied()
        .map(|function| {
            let binding_id = function_header_binding_id(semantic, function);
            let scope_id = binding_id
                .and_then(|binding_id| function_header_scope_id(semantic, function, binding_id));
            let call_arity = binding_id
                .and_then(|binding_id| call_arity_by_binding.get(&binding_id).cloned())
                .unwrap_or_default();

            FunctionHeaderFact {
                function,
                binding_id,
                scope_id,
                call_arity,
            }
        })
        .collect()
}

fn build_function_cli_dispatch_facts(
    semantic: &SemanticModel,
    function_headers: &[FunctionHeaderFact<'_>],
    file: &File,
    source: &str,
) -> FxHashMap<ScopeId, FunctionCliDispatchFacts> {
    let mut facts = FxHashMap::<ScopeId, FunctionCliDispatchFacts>::default();
    let scopes_by_binding = function_headers
        .iter()
        .filter_map(|header| Some((header.binding_id()?, header.function_scope()?)))
        .collect::<FxHashMap<_, _>>();

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
        if !stmt_is_top_level_status_exit(trailing_exit_stmt) {
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
                let Some(binding_id) = visible_function_binding_defined_before_offset(
                    semantic,
                    &name,
                    dispatcher_span.start.offset,
                ) else {
                    continue;
                };
                let Some(scope) = scopes_by_binding.get(&binding_id).copied() else {
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

fn stmt_is_top_level_status_exit(stmt: &Stmt) -> bool {
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

    command
        .code
        .as_ref()
        .is_some_and(word_is_standalone_status_capture)
}

fn build_function_parameter_fallback_spans(
    commands: &[CommandFact<'_>],
    structural_command_ids: &[CommandId],
    source: &str,
) -> Vec<Span> {
    let structural_commands = structural_command_ids
        .iter()
        .copied()
        .map(|id| &commands[id.index()])
        .collect::<Vec<_>>();

    structural_commands
        .windows(2)
        .filter_map(|pair| function_parameter_fallback_span(pair, source))
        .collect()
}

fn build_completion_registered_function_command_flags(
    semantic: &SemanticModel,
    commands: &[CommandFact<'_>],
    lists: &[ListFact<'_>],
    source: &str,
) -> Vec<bool> {
    let registered_scopes =
        build_completion_registered_function_scopes(semantic, commands, lists, source);

    commands
        .iter()
        .map(|command| {
            enclosing_function_scope(semantic, command.span().start.offset)
                .is_some_and(|scope| registered_scopes.contains(&scope))
        })
        .collect()
}

fn build_completion_registered_function_scopes(
    semantic: &SemanticModel,
    commands: &[CommandFact<'_>],
    lists: &[ListFact<'_>],
    source: &str,
) -> FxHashSet<ScopeId> {
    let function_candidates = commands
        .iter()
        .map(|command| completion_registered_function_candidate(semantic, command))
        .collect::<Vec<_>>();
    let mut scopes = FxHashSet::default();

    for list in lists {
        for (index, segment) in list.segments().iter().enumerate() {
            let Some(candidate) = function_candidates[segment.command_id().index()].as_ref() else {
                continue;
            };

            if list.segments()[index + 1..].iter().any(|later_segment| {
                command_registers_completion_function(
                    command_fact(commands, later_segment.command_id()),
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
fn build_function_call_arity_facts<'a>(
    semantic: &SemanticModel,
    functions: &[&FunctionDef],
    commands: &[CommandFact<'a>],
    source: &str,
) -> FxHashMap<BindingId, FunctionCallArityFacts> {
    let mut facts = FxHashMap::<BindingId, FunctionCallArityFacts>::default();
    let mut seen_names = FxHashSet::default();

    for function in functions {
        let Some((name, _)) = function.static_name_entries().next() else {
            continue;
        };
        if !seen_names.insert(name.clone()) {
            continue;
        }

        for command in commands {
            if !command.wrappers().is_empty()
                || command.effective_or_literal_name() != Some(name.as_str())
            {
                continue;
            }
            let Some(name_word) = command.body_name_word() else {
                continue;
            };
            let Some(binding_id) = visible_function_binding_for_call_offset(
                semantic,
                name,
                name_word.span.start.offset,
            ) else {
                continue;
            };
            facts
                .entry(binding_id)
                .or_default()
                .record_call(function_call_arg_count(command, source), name_word.span);
        }
    }

    facts
}

fn function_call_arg_count(command: &CommandFact<'_>, source: &str) -> usize {
    let arg_count = command.body_args().len();
    if arg_count != 0 || !command.redirects().is_empty() || !command.is_nested_word_command() {
        return arg_count;
    }

    let Some(name_word) = command.body_name_word() else {
        return 0;
    };
    let stmt_span = trim_trailing_whitespace_span(command.stmt().span, source);
    let tail = if stmt_span.end.offset > name_word.span.end.offset {
        trim_shell_layout_prefix(&source[name_word.span.end.offset..stmt_span.end.offset])
    } else {
        trim_shell_layout_prefix(&source[name_word.span.end.offset..])
    };
    if tail.is_empty() {
        return 0;
    }
    if matches!(
        tail.as_bytes().first(),
        Some(b')' | b';' | b'|' | b'&' | b'<' | b'>' | b'#' | b'`')
    ) {
        return 0;
    }

    1
}

fn function_header_binding_id(
    semantic: &SemanticModel,
    function: &FunctionDef,
) -> Option<BindingId> {
    let (name, name_span) = function.static_name_entries().next()?;
    semantic
        .function_definitions(name)
        .iter()
        .copied()
        .find(|binding_id| semantic.binding(*binding_id).span == name_span)
}

fn function_header_scope_id(
    semantic: &SemanticModel,
    function: &FunctionDef,
    binding_id: BindingId,
) -> Option<ScopeId> {
    let (name, _) = function.static_name_entries().next()?;
    let binding = semantic.binding(binding_id);

    semantic.scopes().iter().find_map(|scope| {
        let shuck_semantic::ScopeKind::Function(function_scope) = &scope.kind else {
            return None;
        };
        (scope.parent == Some(binding.scope)
            && scope.span == function.body.span
            && function_scope.contains_name(name))
        .then_some(scope.id)
    })
}

fn visible_function_binding_for_call_offset(
    semantic: &SemanticModel,
    name: &Name,
    site_offset: usize,
) -> Option<BindingId> {
    let scopes = semantic
        .ancestor_scopes(semantic.scope_at(site_offset))
        .collect::<Vec<_>>();

    scopes
        .iter()
        .copied()
        .find_map(|scope| {
            semantic
                .function_definitions(name)
                .iter()
                .copied()
                .filter(|candidate| semantic.binding(*candidate).scope == scope)
                .filter(|candidate| semantic.binding(*candidate).span.start.offset < site_offset)
                .max_by_key(|candidate| semantic.binding(*candidate).span.start.offset)
        })
        .or_else(|| {
            scopes.iter().copied().find_map(|scope| {
                semantic
                    .function_definitions(name)
                    .iter()
                    .copied()
                    .filter(|candidate| semantic.binding(*candidate).scope == scope)
                    .min_by_key(|candidate| semantic.binding(*candidate).span.start.offset)
            })
        })
}

fn visible_function_binding_defined_before_offset(
    semantic: &SemanticModel,
    name: &Name,
    site_offset: usize,
) -> Option<BindingId> {
    let scopes = semantic
        .ancestor_scopes(semantic.scope_at(site_offset))
        .collect::<Vec<_>>();

    scopes.iter().copied().find_map(|scope| {
        semantic
            .function_definitions(name)
            .iter()
            .copied()
            .filter(|candidate| semantic.binding(*candidate).scope == scope)
            .filter(|candidate| semantic.binding(*candidate).span.start.offset < site_offset)
            .max_by_key(|candidate| semantic.binding(*candidate).span.start.offset)
    })
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
        Command::Compound(CompoundCommand::BraceGroup(_)) => None,
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

fn collect_terminal_redundant_return_status_spans(function: &FunctionDef, spans: &mut Vec<Span>) {
    collect_terminal_redundant_return_status_spans_in_stmt(&function.body, spans);
}

fn collect_terminal_redundant_return_status_spans_in_stmt(stmt: &Stmt, spans: &mut Vec<Span>) {
    match &stmt.command {
        Command::Compound(CompoundCommand::BraceGroup(commands)) => {
            collect_terminal_redundant_return_status_spans_in_seq(commands, spans);
        }
        Command::Compound(CompoundCommand::If(command)) => {
            collect_terminal_redundant_return_status_spans_in_if(command, spans);
        }
        Command::Simple(_)
        | Command::Decl(_)
        | Command::Builtin(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => {}
    }
}

fn collect_terminal_redundant_return_status_spans_in_if(
    command: &IfCommand,
    spans: &mut Vec<Span>,
) {
    collect_terminal_redundant_return_status_spans_in_seq(&command.then_branch, spans);
    for (_, branch) in &command.elif_branches {
        collect_terminal_redundant_return_status_spans_in_seq(branch, spans);
    }
    if let Some(branch) = &command.else_branch {
        collect_terminal_redundant_return_status_spans_in_seq(branch, spans);
    }
}

fn collect_terminal_redundant_return_status_spans_in_seq(
    commands: &StmtSeq,
    spans: &mut Vec<Span>,
) {
    if let Some(span) = terminal_redundant_return_status_span(commands) {
        spans.push(span);
    }

    let Some(last) = commands.last() else {
        return;
    };
    if last.negated || matches!(last.terminator, Some(StmtTerminator::Background(_))) {
        return;
    }
    collect_terminal_redundant_return_status_spans_in_stmt(last, spans);
}

fn terminal_redundant_return_status_span(commands: &StmtSeq) -> Option<Span> {
    let [.., previous, last] = commands.as_slice() else {
        return None;
    };
    if !stmt_is_terminal_status_propagating_command(previous) {
        return None;
    }
    if last.negated || matches!(last.terminator, Some(StmtTerminator::Background(_))) {
        return None;
    }

    let Command::Builtin(BuiltinCommand::Return(command)) = &last.command else {
        return None;
    };
    if !command.extra_args.is_empty()
        || !command.assignments.is_empty()
        || !last.redirects.is_empty()
    {
        return None;
    }
    let code = command.code.as_ref()?;
    word_is_standalone_status_capture(code).then_some(code.span)
}

fn stmt_is_terminal_status_propagating_command(stmt: &Stmt) -> bool {
    if stmt.negated || matches!(stmt.terminator, Some(StmtTerminator::Background(_))) {
        return false;
    }

    !matches!(stmt.command, Command::Builtin(_))
}

fn build_function_positional_parameter_facts(
    semantic: &SemanticModel,
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
        if let Some(scope) = innermost_nonpersistent_scope_within_function(semantic, offset) {
            local_reset_offsets_by_scope
                .entry(scope)
                .or_default()
                .push(offset);
        }
    }

    for reference in semantic.references() {
        if reference_has_local_positional_reset(
            semantic,
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

            let Some(scope) = enclosing_function_scope(semantic, reference.span.start.offset)
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

        let Some(scope) = enclosing_function_scope(semantic, reference.span.start.offset) else {
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

        if reference_has_local_positional_reset(
            semantic,
            fragment.span().start.offset,
            &local_reset_offsets_by_scope,
        ) {
            continue;
        }

        let Some(scope) = enclosing_function_scope(semantic, fragment.span().start.offset) else {
            continue;
        };

        facts
            .entry(scope)
            .or_default()
            .uses_unprotected_positional_parameters = true;
    }

    for command in commands {
        let Some(scope) =
            enclosing_function_scope_for_positional_reset(semantic, command.span().start.offset)
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

fn enclosing_function_scope(semantic: &SemanticModel, offset: usize) -> Option<ScopeId> {
    let scope = semantic.scope_at(offset);
    semantic.ancestor_scopes(scope).find(|scope| {
        matches!(
            semantic.scope_kind(*scope),
            shuck_semantic::ScopeKind::Function(_)
        )
    })
}

fn enclosing_function_scope_for_positional_reset(
    semantic: &SemanticModel,
    offset: usize,
) -> Option<ScopeId> {
    let scope = semantic.scope_at(offset);

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
    offset: usize,
) -> Option<ScopeId> {
    let scope = semantic.scope_at(offset);

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
    offset: usize,
    local_reset_offsets_by_scope: &FxHashMap<ScopeId, Vec<usize>>,
) -> bool {
    let scope = semantic.scope_at(offset);

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
