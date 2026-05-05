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

    #[cfg(test)]
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
    dispatches: &[CaseCliDispatch],
) -> FxHashMap<ScopeId, FunctionCliDispatchFacts> {
    let mut facts = FxHashMap::<ScopeId, FunctionCliDispatchFacts>::default();
    for dispatch in dispatches {
        facts
            .entry(dispatch.function_scope())
            .or_default()
            .record_dispatch(dispatch.dispatcher_span());
    }
    facts
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
    semantic_analysis: &SemanticAnalysis<'_>,
    commands: &[CommandFact<'_>],
    registered_scopes: &FxHashSet<ScopeId>,
) -> Vec<bool> {
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
    let top_level_candidate_scopes = function_candidates
        .iter()
        .flatten()
        .map(|candidate| candidate.scope)
        .collect::<FxHashSet<_>>();
    let mut scopes = FxHashSet::default();

    for list in lists {
        for (index, segment) in list.segments().iter().enumerate() {
            let Some(candidate) = function_candidates[segment.command_id().index()].as_ref() else {
                continue;
            };

            if list.segments()[index + 1..].iter().any(|later_segment| {
                let later_command =
                    command_fact(commands, command_fact_indices_by_id, later_segment.command_id());
                is_top_level_zsh_entrypoint_registration(semantic, later_command)
                    && command_registers_completion_function(later_command, source, &candidate.name)
            }) {
                scopes.insert(candidate.scope);
            }
        }
    }

    for command in commands {
        let Some(candidate) = function_candidates[command.id().index()].as_ref() else {
            continue;
        };
        if commands.iter().any(|later_command| {
            later_command.span().start.offset > command.span().end.offset
                && is_top_level_zsh_entrypoint_registration(semantic, later_command)
                && later_command.effective_or_literal_name() == Some("compdef")
                && command_registers_completion_function(later_command, source, &candidate.name)
        }) {
            scopes.insert(candidate.scope);
        }
    }

    for scope in semantic.scopes() {
        if !top_level_candidate_scopes.contains(&scope.id) {
            continue;
        }
        let ScopeKind::Function(FunctionScopeKind::Named(names)) = &scope.kind else {
            continue;
        };
        if commands.iter().any(|command| {
            command.span().start.offset > scope.span.end.offset
                && is_top_level_zsh_entrypoint_registration(semantic, command)
                && command.effective_or_literal_name() == Some("compdef")
                && names.iter().any(|name| {
                    command_registers_completion_function(command, source, name.as_str())
                })
        }) {
            scopes.insert(scope.id);
        }
    }

    scopes
}

#[cfg_attr(shuck_profiling, inline(never))]
fn build_external_entrypoint_function_scopes(
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
            external_entrypoint_function_candidate(semantic, command);
    }

    let mut scopes = FxHashSet::default();
    for candidate in function_candidates.iter().flatten() {
        if is_zsh_special_hook_name(&candidate.name) {
            scopes.insert(candidate.scope);
        }
    }

    let mut zsh_widget_functions = FxHashMap::<Box<str>, Box<str>>::default();
    let mut zsh_hook_targets = FxHashSet::<(Box<str>, ZshHookTarget)>::default();
    for command in commands {
        if !is_top_level_zsh_entrypoint_registration(semantic, command) {
            continue;
        }
        match command_zsh_external_entrypoint_action(command, source) {
            Some(ZshExternalEntrypointAction::RegisterWidget { widget, function }) => {
                zsh_widget_functions.insert(widget, function);
            }
            Some(ZshExternalEntrypointAction::UnregisterWidgets { widgets }) => {
                for widget in widgets {
                    zsh_widget_functions.remove(&widget);
                }
            }
            Some(ZshExternalEntrypointAction::RegisterHookFunction { hook, function }) => {
                zsh_hook_targets.insert((hook, ZshHookTarget::Function(function)));
            }
            Some(ZshExternalEntrypointAction::RegisterHookWidget { hook, widget }) => {
                zsh_hook_targets.insert((hook, ZshHookTarget::Widget(widget)));
            }
            Some(ZshExternalEntrypointAction::UnregisterHookFunction { hook, function }) => {
                zsh_hook_targets.remove(&(hook, ZshHookTarget::Function(function)));
            }
            Some(ZshExternalEntrypointAction::UnregisterHookWidget { hook, widget }) => {
                zsh_hook_targets.remove(&(hook, ZshHookTarget::Widget(widget)));
            }
            Some(ZshExternalEntrypointAction::UnregisterHookFunctionPattern { hook, pattern }) => {
                zsh_hook_targets.retain(|(registered_hook, registered_target)| {
                    registered_hook.as_ref() != hook.as_ref()
                        || !registered_target.is_function_matching_pattern(&pattern)
                });
            }
            Some(ZshExternalEntrypointAction::UnregisterHookWidgetPattern { hook, pattern }) => {
                zsh_hook_targets.retain(|(registered_hook, registered_target)| {
                    registered_hook.as_ref() != hook.as_ref()
                        || !registered_target.is_widget_matching_pattern(&pattern)
                });
            }
            None => {}
        }
    }

    for candidate in function_candidates.iter().flatten() {
        if zsh_widget_functions
            .values()
            .any(|name| name.as_ref() == candidate.name.as_ref())
            || zsh_hook_targets
                .iter()
                .any(|(_, target)| target.matches_function(&candidate.name, &zsh_widget_functions))
        {
            scopes.insert(candidate.scope);
        }
    }

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

fn is_top_level_zsh_entrypoint_registration(
    semantic: &SemanticModel,
    command: &CommandFact<'_>,
) -> bool {
    semantic.scope(command.scope()).parent.is_none()
        && !command.is_nested_word_command()
        && !has_structural_parent_command(semantic, command.id())
}

fn has_structural_parent_command(semantic: &SemanticModel, id: CommandId) -> bool {
    let mut current = semantic.syntax_backed_command_parent_id(id);
    while let Some(parent) = current {
        if matches!(
            semantic.command_kind(parent),
            shuck_semantic::CommandKind::Compound(_)
                | shuck_semantic::CommandKind::Function
                | shuck_semantic::CommandKind::AnonymousFunction
        ) {
            return true;
        }
        current = semantic.syntax_backed_command_parent_id(parent);
    }
    false
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
    if !is_top_level_zsh_entrypoint_registration(semantic, command) {
        return None;
    }
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

fn external_entrypoint_function_candidate(
    semantic: &SemanticModel,
    command: &CommandFact<'_>,
) -> Option<CompletionRegisteredFunctionCandidate> {
    let Command::Function(function) = command.command() else {
        return None;
    };
    let (name, _) = function.static_name_entries().next()?;
    let scope = semantic.scope_at(function.body.span.start.offset);
    let name_text = name.as_str();

    Some(CompletionRegisteredFunctionCandidate {
        scope,
        name: name_text.to_owned().into_boxed_str(),
    })
}

fn command_registers_completion_function(
    command: &CommandFact<'_>,
    source: &str,
    expected_name: &str,
) -> bool {
    let command_name = command.effective_or_literal_name();

    if command_name == Some("compdef") {
        for word in command.body_args() {
            let Some(text) = static_word_text(word, source) else {
                continue;
            };
            if text == "--" {
                continue;
            }
            if text.starts_with('-') {
                if text.contains('d') || text.contains('D') {
                    return false;
                }
                continue;
            }
            if text == expected_name {
                return true;
            }
            return false;
        }
        return false;
    }

    if command_name == Some("complete") {
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
    }

    false
}

enum ZshExternalEntrypointAction {
    RegisterWidget { widget: Box<str>, function: Box<str> },
    UnregisterWidgets { widgets: Vec<Box<str>> },
    RegisterHookFunction { hook: Box<str>, function: Box<str> },
    RegisterHookWidget { hook: Box<str>, widget: Box<str> },
    UnregisterHookFunction { hook: Box<str>, function: Box<str> },
    UnregisterHookWidget { hook: Box<str>, widget: Box<str> },
    UnregisterHookFunctionPattern { hook: Box<str>, pattern: Box<str> },
    UnregisterHookWidgetPattern { hook: Box<str>, pattern: Box<str> },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ZshHookTarget {
    Function(Box<str>),
    Widget(Box<str>),
}

impl ZshHookTarget {
    fn matches_function(
        &self,
        function: &str,
        widget_functions: &FxHashMap<Box<str>, Box<str>>,
    ) -> bool {
        match self {
            Self::Function(name) => name.as_ref() == function,
            Self::Widget(widget) => {
                widget.as_ref() == function
                    || widget_functions
                        .get(widget)
                        .is_some_and(|name| name.as_ref() == function)
            }
        }
    }

    fn is_function_matching_pattern(&self, pattern: &str) -> bool {
        match self {
            Self::Function(name) => zsh_hook_function_pattern_matches(pattern, name),
            Self::Widget(_) => false,
        }
    }

    fn is_widget_matching_pattern(&self, pattern: &str) -> bool {
        match self {
            Self::Function(_) => false,
            Self::Widget(name) => zsh_hook_function_pattern_matches(pattern, name),
        }
    }
}

fn command_zsh_external_entrypoint_action(
    command: &CommandFact<'_>,
    source: &str,
) -> Option<ZshExternalEntrypointAction> {
    match command.effective_or_literal_name()? {
        "zle" => zle_external_entrypoint_action(command, source),
        "add-zsh-hook" => add_zsh_hook_external_entrypoint_action(
            command,
            source,
            AddZshHookTargetKind::Function,
        ),
        "add-zle-hook-widget" => add_zsh_hook_external_entrypoint_action(
            command,
            source,
            AddZshHookTargetKind::Widget,
        ),
        _ => None,
    }
}

fn zle_external_entrypoint_action(
    command: &CommandFact<'_>,
    source: &str,
) -> Option<ZshExternalEntrypointAction> {
    let args = static_command_args(command, source)?;

    if let Some(registration_index) = args.iter().position(|arg| arg == "-N") {
        let operands = args[registration_index + 1..]
            .iter()
            .filter(|arg| !arg.starts_with('-'))
            .collect::<Vec<_>>();
        return match operands.as_slice() {
            [widget] => Some(ZshExternalEntrypointAction::RegisterWidget {
                widget: widget.as_str().into(),
                function: widget.as_str().into(),
            }),
            [widget, function, ..] => Some(ZshExternalEntrypointAction::RegisterWidget {
                widget: widget.as_str().into(),
                function: function.as_str().into(),
            }),
            _ => None,
        };
    }

    if let Some(removal_index) = args.iter().position(|arg| arg == "-D") {
        let widgets = args[removal_index + 1..]
            .iter()
            .filter(|arg| !arg.starts_with('-'))
            .map(|arg| arg.as_str().into())
            .collect::<Vec<_>>();
        if !widgets.is_empty() {
            return Some(ZshExternalEntrypointAction::UnregisterWidgets { widgets });
        }
    }

    None
}

fn add_zsh_hook_external_entrypoint_action(
    command: &CommandFact<'_>,
    source: &str,
    target_kind: AddZshHookTargetKind,
) -> Option<ZshExternalEntrypointAction> {
    let args = static_command_args(command, source)?;
    let removal_mode = add_zsh_hook_removal_mode(&args);
    let operands = args
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .collect::<Vec<_>>();
    match operands.as_slice() {
        [hook, function, ..] if removal_mode == Some(AddZshHookRemovalMode::Exact) => match target_kind
        {
            AddZshHookTargetKind::Function => Some(ZshExternalEntrypointAction::UnregisterHookFunction {
                hook: hook.as_str().into(),
                function: function.as_str().into(),
            }),
            AddZshHookTargetKind::Widget => Some(ZshExternalEntrypointAction::UnregisterHookWidget {
                hook: hook.as_str().into(),
                widget: function.as_str().into(),
            }),
        },
        [hook, pattern, ..] if removal_mode == Some(AddZshHookRemovalMode::Pattern) => {
            match target_kind {
                AddZshHookTargetKind::Function => Some(
                    ZshExternalEntrypointAction::UnregisterHookFunctionPattern {
                        hook: hook.as_str().into(),
                        pattern: pattern.as_str().into(),
                    },
                ),
                AddZshHookTargetKind::Widget => Some(
                    ZshExternalEntrypointAction::UnregisterHookWidgetPattern {
                        hook: hook.as_str().into(),
                        pattern: pattern.as_str().into(),
                    },
                ),
            }
        }
        [hook, function, ..] if removal_mode.is_none() => match target_kind {
            AddZshHookTargetKind::Function => Some(ZshExternalEntrypointAction::RegisterHookFunction {
                hook: hook.as_str().into(),
                function: function.as_str().into(),
            }),
            AddZshHookTargetKind::Widget => Some(ZshExternalEntrypointAction::RegisterHookWidget {
                hook: hook.as_str().into(),
                widget: function.as_str().into(),
            }),
        },
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
enum AddZshHookTargetKind {
    Function,
    Widget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddZshHookRemovalMode {
    Exact,
    Pattern,
}

fn add_zsh_hook_removal_mode(args: &[String]) -> Option<AddZshHookRemovalMode> {
    if args.iter().any(|arg| add_zsh_hook_option_contains(arg, 'D')) {
        return Some(AddZshHookRemovalMode::Pattern);
    }
    args.iter()
        .any(|arg| add_zsh_hook_option_contains(arg, 'd'))
        .then_some(AddZshHookRemovalMode::Exact)
}

fn add_zsh_hook_option_contains(arg: &str, flag: char) -> bool {
    arg.starts_with('-') && arg != "--" && arg.chars().skip(1).any(|c| c == flag)
}

fn zsh_hook_function_pattern_matches(pattern: &str, function: &str) -> bool {
    if pattern_contains_unsupported_zsh_metachar(pattern) {
        return true;
    }
    zsh_simple_glob_pattern_matches(pattern.as_bytes(), function.as_bytes())
}

fn zsh_simple_glob_pattern_matches(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }

    match pattern[0] {
        b'*' => {
            zsh_simple_glob_pattern_matches(&pattern[1..], text)
                || (!text.is_empty() && zsh_simple_glob_pattern_matches(pattern, &text[1..]))
        }
        b'?' => !text.is_empty() && zsh_simple_glob_pattern_matches(&pattern[1..], &text[1..]),
        b'[' => zsh_bracket_pattern_matches(pattern, text),
        b'<' if pattern.starts_with(b"<->") => zsh_numeric_pattern_matches(&pattern[3..], text),
        literal => {
            text.first().is_some_and(|byte| *byte == literal)
                && zsh_simple_glob_pattern_matches(&pattern[1..], &text[1..])
        }
    }
}

fn pattern_contains_unsupported_zsh_metachar(pattern: &str) -> bool {
    let mut in_bracket_class = false;
    for byte in pattern.bytes() {
        match byte {
            b'[' if !in_bracket_class => in_bracket_class = true,
            b']' if in_bracket_class => in_bracket_class = false,
            b'(' | b')' | b'|' | b'~' | b'#' if !in_bracket_class => return true,
            _ => {}
        }
    }
    false
}

fn zsh_numeric_pattern_matches(pattern_after_numeric: &[u8], text: &[u8]) -> bool {
    let digit_count = text
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    (1..=digit_count).any(|consumed| {
        zsh_simple_glob_pattern_matches(pattern_after_numeric, &text[consumed..])
    })
}

fn zsh_bracket_pattern_matches(pattern: &[u8], text: &[u8]) -> bool {
    let Some(&candidate) = text.first() else {
        return false;
    };
    let Some(close_index) = pattern.iter().position(|byte| *byte == b']') else {
        return candidate == b'[' && zsh_simple_glob_pattern_matches(&pattern[1..], &text[1..]);
    };
    if close_index == 1 {
        return candidate == b'[' && zsh_simple_glob_pattern_matches(&pattern[1..], &text[1..]);
    }

    let class = &pattern[1..close_index];
    let (negated, class) = match class {
        [b'!' | b'^', rest @ ..] => (true, rest),
        _ => (false, class),
    };
    let matched = zsh_bracket_class_contains(class, candidate);
    if matched != negated {
        zsh_simple_glob_pattern_matches(&pattern[close_index + 1..], &text[1..])
    } else {
        false
    }
}

fn zsh_bracket_class_contains(class: &[u8], candidate: u8) -> bool {
    let mut index = 0;
    while index < class.len() {
        if index + 2 < class.len() && class[index + 1] == b'-' {
            if class[index] <= candidate && candidate <= class[index + 2] {
                return true;
            }
            index += 3;
        } else {
            if class[index] == candidate {
                return true;
            }
            index += 1;
        }
    }
    false
}

fn static_command_args(command: &CommandFact<'_>, source: &str) -> Option<Vec<String>> {
    command
        .body_args()
        .iter()
        .map(|word| static_word_text(word, source).map(|text| text.into_owned()))
        .collect()
}

fn is_zsh_special_hook_name(name: &str) -> bool {
    matches!(
        name,
        "precmd"
            | "preexec"
            | "chpwd"
            | "periodic"
            | "zshaddhistory"
            | "zsh_directory_name"
            | "zshexit"
    )
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
    commands: &[CommandFact<'_>],
    positional_parameter_fragments: &[PositionalParameterFragmentFact],
) -> FxHashMap<ScopeId, FunctionPositionalParameterFacts> {
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
        if let Some(scope) = semantic.innermost_transient_scope_within_function(command.scope()) {
            local_reset_offsets_by_scope
                .entry(scope)
                .or_default()
                .push(offset);
        }
    }

    let mut facts = semantic
        .function_positional_reference_summary(&local_reset_offsets_by_scope)
        .into_iter()
        .map(|(scope, summary)| {
            (
                scope,
                FunctionPositionalParameterFacts {
                    required_arg_count: summary.required_arg_count(),
                    uses_unprotected_positional_parameters: summary
                        .uses_unprotected_positional_parameters(),
                    resets_positional_parameters: false,
                },
            )
        })
        .collect::<FxHashMap<_, _>>();

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

        let Some(scope) = semantic.enclosing_function_scope(fragment_scope) else {
            continue;
        };

        facts
            .entry(scope)
            .or_default()
            .uses_unprotected_positional_parameters = true;
    }

    for command in commands {
        let Some(scope) = semantic.enclosing_function_scope_without_transient_boundary(command.scope())
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

fn reference_has_local_positional_reset(
    semantic: &SemanticModel,
    scope: ScopeId,
    offset: usize,
    local_reset_offsets_by_scope: &FxHashMap<ScopeId, Vec<usize>>,
) -> bool {
    semantic
        .transient_ancestor_scopes_within_function(scope)
        .any(|transient_scope| {
            local_reset_offsets_by_scope
                .get(&transient_scope)
                .is_some_and(|offsets| offsets.iter().any(|reset_offset| *reset_offset < offset))
        })
}
