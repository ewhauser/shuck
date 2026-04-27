use super::*;

pub(super) fn parse_find_exec_shell_command(
    command: &Command,
    source: &str,
) -> Option<FindExecShellCommandFacts> {
    let Command::Simple(command) = command else {
        return None;
    };

    let words = simple_command_body_words(command, source).collect::<Vec<_>>();
    let mut shell_command_spans = Vec::new();
    let mut index = 0usize;

    while index < words.len() {
        let Some(action) = static_word_text(words[index], source) else {
            index += 1;
            continue;
        };
        if !matches!(action.as_ref(), "-exec" | "-execdir" | "-ok" | "-okdir") {
            index += 1;
            continue;
        }

        let Some(command_name_index) = words.get(index + 1).map(|_| index + 1) else {
            break;
        };
        let argument_start = command_name_index;
        let terminator_index = find_exec_terminator_index(&words[argument_start..], source)
            .map(|offset| argument_start + offset);
        let argument_end = terminator_index.unwrap_or(words.len());

        if matches!(action.as_ref(), "-exec" | "-execdir")
            && let Some(segment) = words.get(argument_start..argument_end)
        {
            shell_command_spans.extend(find_exec_shell_command_spans(segment, source));
        }

        index = terminator_index.map_or(words.len(), |terminator_index| terminator_index + 1);
    }

    (!shell_command_spans.is_empty()).then_some(FindExecShellCommandFacts {
        shell_command_spans: shell_command_spans.into_boxed_slice(),
    })
}

fn find_exec_shell_command_spans(args: &[&Word], source: &str) -> Vec<Span> {
    let Some(normalized) = command::normalize_command_words(args, source) else {
        return Vec::new();
    };
    if normalized.has_wrapper(WrapperKind::FindExec)
        || normalized.has_wrapper(WrapperKind::FindExecDir)
    {
        return Vec::new();
    }
    let Some(shell_name) = normalized
        .effective_name
        .as_deref()
        .map(|name| name.rsplit('/').next().unwrap_or(name))
    else {
        return Vec::new();
    };
    if !matches!(shell_name, "sh" | "bash" | "dash" | "ksh") {
        return Vec::new();
    }

    normalized
        .body_args()
        .windows(2)
        .filter_map(|pair| {
            let flag = static_word_text(pair[0], source)?;
            if !shell_flag_contains_command_string(flag.as_ref()) {
                return None;
            }
            let script = pair[1];
            script
                .span
                .slice(source)
                .contains("{}")
                .then_some(script.span)
        })
        .collect()
}

pub(super) fn parse_find_exec_argument_word_spans(command: &Command, source: &str) -> Vec<Span> {
    let Command::Simple(command) = command else {
        return Vec::new();
    };

    let words = simple_command_body_words(command, source).collect::<Vec<_>>();
    let mut spans = Vec::new();
    let mut index = 0usize;

    while index < words.len() {
        let Some(action) = static_word_text(words[index], source) else {
            index += 1;
            continue;
        };
        if !matches!(action.as_ref(), "-exec" | "-ok" | "-execdir" | "-okdir") {
            index += 1;
            continue;
        }

        let Some(command_name_index) = words.get(index + 1).map(|_| index + 1) else {
            break;
        };
        let argument_start = command_name_index;
        let terminator_index = find_exec_terminator_index(&words[argument_start..], source)
            .map(|offset| argument_start + offset);
        let argument_end = terminator_index.unwrap_or(words.len());

        spans.extend(
            words[argument_start..argument_end]
                .iter()
                .map(|word| word.span),
        );

        index = terminator_index.map_or(words.len(), |terminator_index| terminator_index + 1);
    }

    spans
}

fn find_exec_terminator_index(args: &[&Word], source: &str) -> Option<usize> {
    let semicolon_terminator_index = args
        .iter()
        .position(|word| is_find_exec_semicolon_terminator(word, source));
    let plus_terminator_index = args
        .iter()
        .enumerate()
        .filter_map(|(index, word)| {
            (index > 0
                && static_word_text(word, source).as_deref() == Some("+")
                && static_word_text(args[index - 1], source).as_deref() == Some("{}"))
            .then_some(index)
        })
        .next();
    match (semicolon_terminator_index, plus_terminator_index) {
        (Some(semicolon_index), Some(plus_index)) => Some(semicolon_index.min(plus_index)),
        (Some(semicolon_index), None) => Some(semicolon_index),
        (None, Some(plus_index)) => Some(plus_index),
        (None, None) => None,
    }
}

fn is_find_exec_semicolon_terminator(word: &Word, source: &str) -> bool {
    match static_word_text(word, source).as_deref() {
        Some(";") => true,
        Some("\\;") => classify_word(word, source).quote == WordQuote::Unquoted,
        _ => false,
    }
}

pub(super) fn find_command_args<'a>(
    command: &'a Command,
    normalized: &'a NormalizedCommand<'a>,
    source: &'a str,
) -> impl Iterator<Item = &'a Word> + 'a {
    if normalized.literal_name.as_deref() == Some("find")
        && let Command::Simple(command) = command
    {
        return EitherFindCommandArgs::Simple(simple_command_body_words(command, source).skip(1));
    }

    EitherFindCommandArgs::Normalized(normalized.body_args().iter().copied())
}

enum EitherFindCommandArgs<I, J> {
    Simple(I),
    Normalized(J),
}

impl<'a, I, J> Iterator for EitherFindCommandArgs<I, J>
where
    I: Iterator<Item = &'a Word>,
    J: Iterator<Item = &'a Word>,
{
    type Item = &'a Word;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Simple(iter) => iter.next(),
            Self::Normalized(iter) => iter.next(),
        }
    }
}

pub(super) fn parse_find_command<'a>(
    args: impl IntoIterator<Item = &'a Word>,
    source: &str,
) -> FindCommandFacts {
    let mut has_print0 = false;
    let mut has_formatted_output_action = false;
    let mut or_without_grouping_spans = Vec::new();
    let mut glob_pattern_operand_spans = Vec::new();
    let mut group_stack = vec![FindGroupState::default()];
    let mut pending_argument: Option<FindPendingArgument> = None;

    for word in args {
        let Some(text) = static_word_text(word, source) else {
            if let Some(state) = pending_argument {
                if state.expects_pattern_operand()
                    && !word_spans::word_unquoted_glob_pattern_spans(word, source).is_empty()
                {
                    glob_pattern_operand_spans.push(word.span);
                }
                pending_argument = state.after_consuming_dynamic();
            }
            continue;
        };

        if let Some(state) = pending_argument {
            if state.expects_pattern_operand()
                && !word_spans::word_unquoted_glob_pattern_spans(word, source).is_empty()
            {
                glob_pattern_operand_spans.push(word.span);
            }
            pending_argument = state.after_consuming(text.as_ref());
            continue;
        }

        if text == "-print0" {
            has_print0 = true;
        }

        if is_find_group_open_token(text.as_ref()) {
            group_stack.push(FindGroupState::default());
            continue;
        }

        if is_find_group_close_token(text.as_ref()) {
            if let Some(child) = (group_stack.len() > 1).then(|| group_stack.pop()).flatten() {
                let Some(parent) = group_stack.last_mut() else {
                    unreachable!("group stack retains the root frame");
                };
                parent.incorporate_group(child);
            }
            continue;
        }

        let Some(state) = group_stack.last_mut() else {
            unreachable!("group stack retains the root frame");
        };

        if is_find_or_token(text.as_ref()) {
            state.note_or();
            continue;
        }

        if is_find_and_token(text.as_ref()) {
            state.note_and();
            continue;
        }

        if is_find_branch_action_token(text.as_ref()) {
            if matches!(text.as_ref(), "-fprint0" | "-printf" | "-fprintf") {
                has_formatted_output_action = true;
            }
            state.note_action(
                word.span,
                is_find_reportable_action_token(text.as_ref()),
                &mut or_without_grouping_spans,
            );
            pending_argument = find_pending_argument(text.as_ref());
            continue;
        }

        if is_find_predicate_token(text.as_ref()) {
            state.note_predicate();
            pending_argument = find_pending_argument(text.as_ref());
        }
    }

    FindCommandFacts {
        has_print0,
        has_formatted_output_action,
        or_without_grouping_spans: or_without_grouping_spans.into_boxed_slice(),
        glob_pattern_operand_spans: glob_pattern_operand_spans.into_boxed_slice(),
    }
}

#[derive(Debug, Clone, Copy)]
enum FindPendingArgument {
    Words {
        remaining: usize,
        pattern_operand: bool,
    },
    UntilExecTerminator,
}

impl FindPendingArgument {
    fn after_consuming(self, token: &str) -> Option<Self> {
        match self {
            Self::Words {
                remaining,
                pattern_operand: _,
            } => remaining.checked_sub(1).and_then(|next| {
                (next > 0).then_some(Self::Words {
                    remaining: next,
                    pattern_operand: false,
                })
            }),
            Self::UntilExecTerminator => {
                (!matches!(token, ";" | "\\;" | "+")).then_some(Self::UntilExecTerminator)
            }
        }
    }

    fn after_consuming_dynamic(self) -> Option<Self> {
        match self {
            Self::Words {
                remaining,
                pattern_operand: _,
            } => remaining.checked_sub(1).and_then(|next| {
                (next > 0).then_some(Self::Words {
                    remaining: next,
                    pattern_operand: false,
                })
            }),
            Self::UntilExecTerminator => Some(Self::UntilExecTerminator),
        }
    }

    fn expects_pattern_operand(self) -> bool {
        matches!(
            self,
            Self::Words {
                pattern_operand: true,
                ..
            }
        )
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct FindGroupState {
    saw_or: bool,
    saw_action_before_current_branch: bool,
    current_branch_has_predicate: bool,
    current_branch_has_explicit_and: bool,
    has_any_predicate: bool,
    has_any_action: bool,
}

impl FindGroupState {
    fn current_branch_can_bind_action(&self) -> bool {
        !self.current_branch_has_explicit_and && self.current_branch_has_predicate
    }

    fn note_or(&mut self) {
        self.saw_or = true;
        self.current_branch_has_predicate = false;
        self.current_branch_has_explicit_and = false;
    }

    fn note_and(&mut self) {
        self.current_branch_has_explicit_and = true;
    }

    fn note_predicate(&mut self) {
        self.current_branch_has_predicate = true;
        self.has_any_predicate = true;
    }

    fn note_action(&mut self, span: Span, reportable: bool, spans: &mut Vec<Span>) {
        if reportable
            && self.saw_or
            && !self.saw_action_before_current_branch
            && self.current_branch_can_bind_action()
        {
            spans.push(span);
        }

        self.saw_action_before_current_branch = true;
        self.has_any_action = true;
    }

    fn incorporate_group(&mut self, child: Self) {
        if child.has_any_predicate {
            self.note_predicate();
        }

        if child.has_any_action {
            self.saw_action_before_current_branch = true;
            self.has_any_action = true;
        }
    }
}

fn is_find_group_open_token(token: &str) -> bool {
    matches!(token, "(" | "\\(" | "-(")
}

fn is_find_group_close_token(token: &str) -> bool {
    matches!(token, ")" | "\\)" | "-)")
}

fn is_find_or_token(token: &str) -> bool {
    matches!(token, "-o" | "-or")
}

fn is_find_and_token(token: &str) -> bool {
    matches!(token, "-a" | "-and" | ",")
}

fn is_find_action_token(token: &str) -> bool {
    matches!(
        token,
        "-delete"
            | "-exec"
            | "-execdir"
            | "-ok"
            | "-okdir"
            | "-print"
            | "-print0"
            | "-printf"
            | "-ls"
            | "-fls"
            | "-fprint"
            | "-fprint0"
            | "-fprintf"
    )
}

fn is_find_branch_action_token(token: &str) -> bool {
    is_find_reportable_action_token(token) || matches!(token, "-prune" | "-quit")
}

fn is_find_reportable_action_token(token: &str) -> bool {
    is_find_action_token(token)
}

fn find_pending_argument(token: &str) -> Option<FindPendingArgument> {
    match token {
        "-fls" | "-fprint" | "-fprint0" | "-printf" => Some(FindPendingArgument::Words {
            remaining: 1,
            pattern_operand: false,
        }),
        "-fprintf" => Some(FindPendingArgument::Words {
            remaining: 2,
            pattern_operand: false,
        }),
        "-exec" | "-execdir" | "-ok" | "-okdir" => Some(FindPendingArgument::UntilExecTerminator),
        "-amin" | "-anewer" | "-atime" | "-cmin" | "-cnewer" | "-context" | "-fstype" | "-gid"
        | "-group" | "-ilname" | "-iname" | "-inum" | "-ipath" | "-iregex" | "-links"
        | "-lname" | "-maxdepth" | "-mindepth" | "-mmin" | "-mtime" | "-name" | "-newer"
        | "-path" | "-perm" | "-regex" | "-samefile" | "-size" | "-type" | "-uid" | "-used"
        | "-user" | "-wholename" | "-iwholename" | "-xtype" | "-files0-from" => {
            Some(FindPendingArgument::Words {
                remaining: 1,
                pattern_operand: is_find_pattern_predicate_token(token),
            })
        }
        token if token.starts_with("-newer") && token.len() > "-newer".len() => {
            Some(FindPendingArgument::Words {
                remaining: 1,
                pattern_operand: false,
            })
        }
        _ => None,
    }
}

fn is_find_pattern_predicate_token(token: &str) -> bool {
    matches!(
        token,
        "-name"
            | "-iname"
            | "-path"
            | "-ipath"
            | "-regex"
            | "-iregex"
            | "-lname"
            | "-ilname"
            | "-wholename"
            | "-iwholename"
    )
}

fn is_find_predicate_token(token: &str) -> bool {
    token.starts_with('-')
        && !is_find_branch_action_token(token)
        && !is_find_or_token(token)
        && !is_find_and_token(token)
        && !is_find_group_open_token(token)
        && !is_find_group_close_token(token)
        && !matches!(token, "-not")
}
