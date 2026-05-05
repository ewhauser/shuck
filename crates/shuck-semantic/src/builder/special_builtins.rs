use super::*;

impl<'a, 'observer> SemanticModelBuilder<'a, 'observer> {
    pub(super) fn classify_special_simple_command(
        &mut self,
        name: &Name,
        normalized: &NormalizedCommand<'a>,
        command_span: Span,
        flow: FlowState,
    ) {
        let args = normalized.body_args();
        let name_span = normalized.body_word_span().unwrap_or(command_span);
        match name.as_str() {
            "read" => {
                let read_assigns_array = read_assigns_array(args, self.source);
                for (target_index, (argument, span)) in
                    iter_read_targets(args, self.source).into_iter().enumerate()
                {
                    let target_attributes = if read_assigns_array && target_index == 0 {
                        BindingAttributes::ARRAY
                    } else {
                        BindingAttributes::empty()
                    };
                    self.add_binding(
                        &argument,
                        BindingKind::ReadTarget,
                        self.current_scope(),
                        span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: span,
                            kind: BuiltinBindingTargetKind::Read,
                        },
                        target_attributes,
                    );
                }
                for implicit_read in
                    self.runtime
                        .implicit_reads_for_simple_command(name, args, self.source)
                {
                    let implicit_name = Name::from(*implicit_read);
                    self.add_reference_if_bound(
                        &implicit_name,
                        ReferenceKind::ImplicitRead,
                        command_span,
                    );
                }
            }
            "mapfile" | "readarray" => match mapfile_target(args, self.source) {
                Some(MapfileTarget::Explicit(argument, span)) => {
                    self.add_binding(
                        &argument,
                        BindingKind::MapfileTarget,
                        self.current_scope(),
                        span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: span,
                            kind: BuiltinBindingTargetKind::Mapfile,
                        },
                        BindingAttributes::ARRAY,
                    );
                }
                Some(MapfileTarget::Implicit) => {
                    self.add_binding(
                        &Name::from("MAPFILE"),
                        BindingKind::MapfileTarget,
                        self.current_scope(),
                        name_span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: name_span,
                            kind: BuiltinBindingTargetKind::Mapfile,
                        },
                        BindingAttributes::ARRAY,
                    );
                }
                None => {}
            },
            "printf" => {
                if let Some((argument, span)) = printf_v_target(args, self.source) {
                    self.add_binding(
                        &argument,
                        BindingKind::PrintfTarget,
                        self.current_scope(),
                        span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: span,
                            kind: BuiltinBindingTargetKind::Printf,
                        },
                        BindingAttributes::empty(),
                    );
                }
            }
            "getopts" => {
                if let Some((argument, span)) = getopts_target(args, self.source) {
                    self.add_binding(
                        &argument,
                        BindingKind::GetoptsTarget,
                        self.current_scope(),
                        span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: span,
                            kind: BuiltinBindingTargetKind::Getopts,
                        },
                        BindingAttributes::empty(),
                    );
                }
            }
            "zparseopts" if self.shell_profile.dialect == shuck_parser::ShellDialect::Zsh => {
                for (argument, span, attributes) in zparseopts_targets(args, self.source) {
                    self.add_binding(
                        &argument,
                        BindingKind::ZparseoptsTarget,
                        self.current_scope(),
                        span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: span,
                            kind: BuiltinBindingTargetKind::Zparseopts,
                        },
                        attributes,
                    );
                }
            }
            "zstyle" if self.shell_profile.dialect == shuck_parser::ShellDialect::Zsh => {
                if let Some((argument, span, mut attributes)) = zstyle_target(args, self.source) {
                    if attributes.contains(BindingAttributes::ARRAY)
                        && self
                            .resolve_reference(&argument, self.current_scope(), span.start.offset)
                            .map(|binding_id| {
                                let binding = &self.bindings[binding_id.index()];
                                binding.attributes.contains(BindingAttributes::ASSOC)
                                    && !self.binding_was_cleared_before_lookup(
                                        binding,
                                        self.current_scope(),
                                        span.start.offset,
                                    )
                            })
                            .unwrap_or(false)
                    {
                        attributes |= BindingAttributes::ASSOC;
                    }
                    self.add_binding(
                        &argument,
                        BindingKind::ReadTarget,
                        self.current_scope(),
                        span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: span,
                            kind: BuiltinBindingTargetKind::Zstyle,
                        },
                        attributes,
                    );
                }
            }
            "_describe" if self.shell_profile.dialect == shuck_parser::ShellDialect::Zsh => {
                for (argument, span) in describe_array_names(args, self.source, |word| {
                    self.describe_dynamic_start(word)
                }) {
                    self.add_reference_if_bound(&argument, ReferenceKind::ImplicitRead, span);
                }
            }
            "_arguments" if self.shell_profile.dialect == shuck_parser::ShellDialect::Zsh => {
                for (argument, attributes) in
                    zsh_arguments_targets(zsh_arguments_has_state_action(args, self.source))
                {
                    self.add_binding(
                        &argument,
                        BindingKind::ReadTarget,
                        self.current_scope(),
                        name_span,
                        BindingOrigin::BuiltinTarget {
                            definition_span: name_span,
                            kind: BuiltinBindingTargetKind::ZshArguments,
                        },
                        attributes,
                    );
                }
            }
            "let" => self.record_let_arithmetic_assignment_targets(args),
            "eval" => self.record_eval_argument_references(args),
            "trap" => self.record_trap_action_references(args),
            "source" | "." => {
                if normalized.wrappers.is_empty()
                    && let Some(argument) = args.first().copied()
                {
                    let source_span = self.command_stack.last().copied().unwrap_or(command_span);
                    let kind = self.classify_source_ref(command_span.line(), argument);
                    self.source_refs.push(SourceRef {
                        diagnostic_class: classify_source_ref_diagnostic_class(
                            argument,
                            self.source,
                            &kind,
                        ),
                        kind,
                        span: source_span,
                        path_span: argument.span,
                        resolution: SourceRefResolution::Unchecked,
                        explicitly_provided: false,
                    });
                }
            }
            "unset" => self.record_unset_variable_targets(args, flow),
            "integer" if self.shell_profile.dialect == shuck_parser::ShellDialect::Zsh => {
                self.visit_simple_declaration_command(name.as_str(), args, command_span, flow);
            }
            "export" | "local" | "declare" | "typeset" | "readonly" => {
                self.visit_simple_declaration_command(name.as_str(), args, command_span, flow);
            }
            _ if name.as_str().starts_with("DEFINE_") => {
                self.visit_command_defined_variable(args);
            }
            _ => {}
        }
    }

    pub(super) fn record_trap_action_references(&mut self, args: &[&'a Word]) {
        let Some(argument) = trap_action_argument(args, self.source) else {
            return;
        };

        let mut seen = FxHashSet::default();
        for name in trap_action_reference_names(argument, self.source) {
            if seen.insert(name.clone()) {
                self.add_reference(&name, ReferenceKind::TrapAction, argument.span);
            }
        }
    }

    pub(super) fn record_let_arithmetic_assignment_targets(&mut self, args: &[&'a Word]) {
        for argument in args {
            let Some((name, span)) = let_arithmetic_assignment_target(argument, self.source) else {
                continue;
            };
            self.add_binding(
                &name,
                BindingKind::ArithmeticAssignment,
                self.current_scope(),
                span,
                BindingOrigin::ArithmeticAssignment {
                    definition_span: span,
                    target_span: span,
                },
                BindingAttributes::empty(),
            );
        }
    }

    pub(super) fn visit_simple_declaration_command(
        &mut self,
        command_name: &str,
        args: &[&'a Word],
        command_span: Span,
        flow: FlowState,
    ) {
        let Some(builtin) = declaration_builtin_name(command_name) else {
            return;
        };

        let mut flags = FxHashSet::default();
        apply_implicit_declaration_flags(command_name, &mut flags);
        let mut global_flag_enabled = false;
        let mut name_operands_are_function_names = false;
        let mut parsing_options = true;
        let mut operands = Vec::new();

        let argument_groups = contiguous_word_groups(args);
        for arguments in argument_groups {
            let Some(argument) = arguments.first().copied() else {
                continue;
            };
            let argument_span = word_group_span(arguments);
            if parsing_options {
                if arguments.len() == 1
                    && let Some(text) = static_word_text(argument, self.source)
                {
                    if text == "--" {
                        parsing_options = false;
                        continue;
                    }

                    if simple_declaration_option_word(&text) {
                        update_simple_declaration_flags(
                            &text,
                            &mut flags,
                            &mut global_flag_enabled,
                            &mut name_operands_are_function_names,
                        );
                        operands.push(simple_declaration_flag_operand(argument, text.as_ref()));
                        continue;
                    }
                }

                parsing_options = false;
            }

            if name_operands_are_function_names {
                operands.push(DeclarationOperand::DynamicWord {
                    span: argument_span,
                });
                continue;
            }

            let explicit_array_kind = declaration_explicit_array_kind(&flags);
            if let Some(assignment) =
                parse_simple_declaration_assignment(arguments, self.source, explicit_array_kind)
            {
                let (scope, mut attributes) = self.simple_declaration_scope_and_attributes(
                    builtin,
                    &flags,
                    global_flag_enabled,
                    flow,
                );
                attributes |= BindingAttributes::DECLARATION_INITIALIZED;
                if assignment.array_like {
                    attributes |= BindingAttributes::ARRAY;
                }
                if flags.contains(&'p') {
                    attributes |= BindingAttributes::EXTERNALLY_CONSUMED;
                }
                let kind = if attributes.contains(BindingAttributes::NAMEREF) {
                    BindingKind::Nameref
                } else {
                    BindingKind::Declaration(builtin)
                };
                let origin = BindingOrigin::Assignment {
                    definition_span: assignment.target_span,
                    value: assignment.value_origin,
                };
                self.add_binding(
                    &assignment.name,
                    kind,
                    scope,
                    assignment.name_span,
                    origin,
                    attributes,
                );
                operands.push(DeclarationOperand::Assignment {
                    name: assignment.name,
                    operand_span: argument_span,
                    target_span: assignment.target_span,
                    name_span: assignment.name_span,
                    value_span: assignment.value_span,
                    append: assignment.append,
                    value_origin: assignment.value_origin,
                    has_command_substitution: assignment.has_command_substitution,
                    has_command_or_process_substitution: assignment
                        .has_command_or_process_substitution,
                });
                continue;
            }

            if arguments.len() != 1 || static_word_text(argument, self.source).is_none() {
                operands.push(DeclarationOperand::DynamicWord {
                    span: argument_span,
                });
                continue;
            }

            if let Some((name, span)) = named_target_word(argument, self.source) {
                self.visit_simple_name_only_declaration_operand(
                    builtin,
                    &flags,
                    global_flag_enabled,
                    flow,
                    &name,
                    span,
                );
                operands.push(DeclarationOperand::Name { name, span });
            } else {
                operands.push(DeclarationOperand::DynamicWord {
                    span: argument_span,
                });
            }
        }

        self.declarations.push(Declaration {
            builtin,
            span: command_span,
            operands,
        });
    }

    pub(super) fn visit_command_defined_variable(&mut self, args: &[&Word]) {
        let Some((flag_name, span)) = args
            .first()
            .copied()
            .and_then(|word| named_target_word(word, self.source))
        else {
            return;
        };
        let generated = Name::from(format!("FLAGS_{}", flag_name.as_str()));
        self.add_binding(
            &generated,
            BindingKind::Declaration(DeclarationBuiltin::Declare),
            self.current_scope(),
            span,
            BindingOrigin::Declaration {
                definition_span: span,
            },
            BindingAttributes::empty(),
        );
    }

    pub(super) fn record_eval_argument_references(&mut self, args: &[&Word]) {
        for argument in args.iter().copied() {
            for (name, span) in eval_argument_reference_names(argument, self.source) {
                self.add_reference_if_bound(&name, ReferenceKind::ImplicitRead, span);
            }
        }
    }

    pub(super) fn record_unset_variable_targets(&mut self, args: &[&Word], flow: FlowState) {
        if flow.conditionally_executed {
            return;
        }

        let mut function_flag_seen = false;
        let mut variable_flag_seen = false;
        let mut nameref_mode = false;
        let mut parsing_options = true;

        for argument in args.iter().copied() {
            let Some(text) = static_word_text(argument, self.source) else {
                if parsing_options {
                    return;
                }
                parsing_options = false;
                continue;
            };

            if parsing_options {
                if text == "--" {
                    parsing_options = false;
                    continue;
                }

                if text.starts_with('-') && text != "-" {
                    let flags = text.trim_start_matches('-');
                    if !unset_flags_are_valid(flags) {
                        return;
                    }
                    for flag in flags.chars() {
                        match flag {
                            'f' => {
                                if variable_flag_seen {
                                    return;
                                }
                                function_flag_seen = true;
                            }
                            'v' => {
                                if function_flag_seen {
                                    return;
                                }
                                variable_flag_seen = true;
                            }
                            'n' => {
                                nameref_mode = true;
                            }
                            _ => unreachable!("invalid unset flag already filtered"),
                        }
                    }
                    continue;
                }

                parsing_options = false;
            }

            if function_flag_seen || !is_name(&text) {
                continue;
            }

            if nameref_mode {
                let name = Name::from(text.as_ref());
                let Some(binding_id) =
                    self.resolve_reference(&name, self.current_scope(), argument.span.start.offset)
                else {
                    continue;
                };
                let binding = &self.bindings[binding_id.index()];
                if !binding.attributes.contains(BindingAttributes::NAMEREF)
                    && !matches!(binding.kind, BindingKind::Nameref)
                {
                    continue;
                }
            }

            self.cleared_variables
                .entry((self.current_scope(), Name::from(text.as_ref())))
                .or_default()
                .push(argument.span.start.offset);
        }
    }

    fn describe_dynamic_start(&self, word: &Word) -> DescribeDynamicStart {
        let Some(name) = standalone_parameter_name(word) else {
            return DescribeDynamicStart::Unknown;
        };
        let Some(binding_id) =
            self.resolve_reference(&name, self.current_scope(), word.span.start.offset)
        else {
            return DescribeDynamicStart::Unknown;
        };
        let binding = &self.bindings[binding_id.index()];
        let Some(value) = static_scalar_assignment_value(binding, self.source) else {
            return DescribeDynamicStart::Unknown;
        };

        match value.as_str() {
            "-t" => DescribeDynamicStart::OptionWithValue,
            "-o" | "-O" => DescribeDynamicStart::OptionWithoutValue,
            _ if value.starts_with('-') && value != "-" => DescribeDynamicStart::Unknown,
            _ => DescribeDynamicStart::Descriptor,
        }
    }
}

fn zstyle_target(args: &[&Word], source: &str) -> Option<(Name, Span, BindingAttributes)> {
    let mut index = 0usize;
    let mut attributes = None;
    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };
        if text == "--" {
            index += 1;
            break;
        }
        let Some(flags) = text.strip_prefix('-') else {
            break;
        };
        if flags.is_empty() || flags.starts_with('-') {
            break;
        }
        for flag in flags.chars() {
            match flag {
                'a' => attributes = Some(BindingAttributes::ARRAY),
                'b' | 's' => attributes = Some(BindingAttributes::empty()),
                'q' => {}
                _ => return None,
            }
        }
        index += 1;
    }

    let attributes = attributes?;
    args.get(index + 2)
        .and_then(|word| named_target_word(word, source))
        .map(|(name, span)| (name, span, attributes))
}

fn zsh_arguments_targets(include_state_targets: bool) -> Vec<(Name, BindingAttributes)> {
    let mut targets = vec![
        (Name::from("context"), BindingAttributes::ARRAY),
        (Name::from("line"), BindingAttributes::ARRAY),
        (
            Name::from("opt_args"),
            BindingAttributes::ARRAY | BindingAttributes::ASSOC,
        ),
    ];
    if include_state_targets {
        targets.push((Name::from("state"), BindingAttributes::ARRAY));
        targets.push((Name::from("state_descr"), BindingAttributes::ARRAY));
    }
    targets
}

fn zsh_arguments_has_state_action(args: &[&Word], source: &str) -> bool {
    args.iter()
        .filter_map(|word| static_word_text(word, source))
        .any(|text| zsh_argument_spec_has_state_action(&text))
}

fn zsh_argument_spec_has_state_action(text: &str) -> bool {
    let mut bracket_depth = 0usize;
    let mut previous_non_whitespace = None;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '[' {
            bracket_depth += 1;
        } else if ch == ']' {
            bracket_depth = bracket_depth.saturating_sub(1);
        } else if ch == '-'
            && chars.peek() == Some(&'>')
            && bracket_depth == 0
            && previous_non_whitespace == Some(':')
        {
            return true;
        }

        if !ch.is_whitespace() {
            previous_non_whitespace = Some(ch);
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DescribeDynamicStart {
    Descriptor,
    OptionWithoutValue,
    OptionWithValue,
    Unknown,
}

fn describe_array_names(
    args: &[&Word],
    source: &str,
    dynamic_start: impl Fn(&Word) -> DescribeDynamicStart,
) -> Vec<(Name, Span)> {
    let mut index = 0usize;
    let mut first_segment_dynamic_start = None;
    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            first_segment_dynamic_start = Some(dynamic_start(word));
            break;
        };
        if text == "--" {
            index += 1;
            break;
        }
        if !text.starts_with('-') || text == "-" {
            break;
        }
        index += 1;
        if text == "-t" {
            index += 1;
        }
    }

    let mut targets = Vec::new();
    let mut first_group = true;
    while index < args.len() {
        if args
            .get(index)
            .and_then(|word| static_word_text(word, source))
            .as_deref()
            == Some("--")
        {
            index += 1;
            first_group = false;
            continue;
        }

        let segment_start = index;
        let mut segment_end = index;
        while segment_end < args.len()
            && args
                .get(segment_end)
                .and_then(|word| static_word_text(word, source))
                .as_deref()
                != Some("--")
        {
            segment_end += 1;
        }

        let segment_len = segment_end.saturating_sub(segment_start);
        let (target_start, target_count) =
            if first_group && let Some(dynamic_start) = first_segment_dynamic_start {
                match dynamic_start {
                    DescribeDynamicStart::Descriptor => (segment_start + 1, 2),
                    DescribeDynamicStart::OptionWithoutValue => match segment_len {
                        0..=2 => (segment_end, 0),
                        _ => (segment_start + 2, 2),
                    },
                    DescribeDynamicStart::OptionWithValue => match segment_len {
                        0..=3 => (segment_end, 0),
                        _ => (segment_start + 3, 2),
                    },
                    DescribeDynamicStart::Unknown => match segment_len {
                        0 | 1 => (segment_end, 0),
                        2 => (segment_start + 1, 1),
                        _ => (segment_start + 2, 2),
                    },
                }
            } else if first_group {
                (segment_start + 1, 2)
            } else {
                (segment_start, 2)
            };
        for target_index in target_start..(target_start + target_count).min(segment_end) {
            if let Some(target) = args
                .get(target_index)
                .and_then(|word| named_target_word(word, source))
            {
                targets.push(target);
            }
        }
        index = segment_end;
        first_group = false;
    }
    targets
}

fn standalone_parameter_name(word: &Word) -> Option<Name> {
    standalone_parameter_name_from_parts(&word.parts)
}

fn standalone_parameter_name_from_parts(parts: &[WordPartNode]) -> Option<Name> {
    let [part] = parts else {
        return None;
    };

    match &part.kind {
        WordPart::Variable(name) => Some(name.clone()),
        WordPart::Parameter(parameter) => match parameter.bourne() {
            Some(BourneParameterExpansion::Access { reference })
                if reference.subscript.is_none() =>
            {
                Some(reference.name.clone())
            }
            _ => None,
        },
        WordPart::DoubleQuoted { parts, .. } => standalone_parameter_name_from_parts(parts),
        _ => None,
    }
}

fn static_scalar_assignment_value(binding: &Binding, source: &str) -> Option<String> {
    if !matches!(
        binding.origin,
        BindingOrigin::Assignment {
            value: AssignmentValueOrigin::StaticLiteral,
            ..
        }
    ) {
        return None;
    }

    let rest = source.get(binding.span.end.offset..)?;
    let rest = rest.strip_prefix('=')?;
    parse_static_assignment_literal(rest)
}

fn parse_static_assignment_literal(rest: &str) -> Option<String> {
    let rest = rest.trim_start();
    if let Some(rest) = rest.strip_prefix('\'') {
        return rest.split_once('\'').map(|(value, _)| value.to_owned());
    }
    if let Some(rest) = rest.strip_prefix('"') {
        let (value, _) = rest.split_once('"')?;
        if value.contains(['$', '`', '\\']) {
            return None;
        }
        return Some(value.to_owned());
    }

    let value = rest
        .split(|ch: char| ch.is_whitespace() || ch == ';' || ch == '#')
        .next()
        .unwrap_or_default();
    if value.is_empty() || value.starts_with('(') {
        None
    } else {
        Some(value.to_owned())
    }
}
