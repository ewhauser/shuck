use std::{borrow::Cow, sync::Arc};

use shuck_ast::{StaticCommandWrapperTarget, static_command_wrapper_target_index};

use super::{ShellProfile, ZshEmulationMode, ZshOptionState};

#[derive(Debug, Clone)]
pub(crate) struct ZshOptionTimeline {
    pub(super) initial: ZshOptionState,
    pub(super) entries: Arc<[ZshOptionTimelineEntry]>,
}

#[derive(Debug, Clone)]
pub(super) struct ZshOptionTimelineEntry {
    pub(super) offset: usize,
    pub(super) state: ZshOptionState,
}

impl ZshOptionTimeline {
    pub(super) fn build(input: &str, shell_profile: &ShellProfile) -> Option<Self> {
        let initial = *shell_profile.zsh_options()?;
        if !might_mutate_zsh_parser_options(input) {
            return Some(Self {
                initial,
                entries: Arc::from([]),
            });
        }

        let entries = ZshOptionPrescanner::new(input, initial).scan();
        Some(Self {
            initial,
            entries: entries.into(),
        })
    }

    pub(super) fn options_at(&self, offset: usize) -> &ZshOptionState {
        let next_index = self.entries.partition_point(|entry| entry.offset <= offset);
        if next_index == 0 {
            &self.initial
        } else {
            &self.entries[next_index - 1].state
        }
    }
}

fn might_mutate_zsh_parser_options(input: &str) -> bool {
    input.contains("setopt")
        || input.contains("unsetopt")
        || input.contains("emulate")
        || input.contains("set -o")
        || input.contains("set +o")
}

#[derive(Debug, Clone)]
struct ZshOptionPrescanner<'a> {
    input: &'a str,
    offset: usize,
    state: ZshOptionState,
    entries: Vec<ZshOptionTimelineEntry>,
}

#[derive(Debug, Clone)]
enum PrescanToken {
    Word {
        text: String,
        end: usize,
    },
    Separator {
        kind: PrescanSeparator,
        start: usize,
        end: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrescanSeparator {
    Newline,
    Semicolon,
    Pipe,
    Ampersand,
    OpenParen,
    CloseParen,
    OpenBrace,
    CloseBrace,
}

#[derive(Debug, Clone)]
struct PrescanLocalScope {
    saved_state: ZshOptionState,
    brace_depth: usize,
    paren_depth: usize,
    compounds: Vec<PrescanCompound>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrescanCompound {
    If,
    Loop,
    Case,
}

impl PrescanLocalScope {
    fn simple(saved_state: ZshOptionState) -> Self {
        Self {
            saved_state,
            brace_depth: 0,
            paren_depth: 0,
            compounds: Vec::new(),
        }
    }

    fn brace_group(saved_state: ZshOptionState) -> Self {
        Self {
            brace_depth: 1,
            ..Self::simple(saved_state)
        }
    }

    fn subshell(saved_state: ZshOptionState) -> Self {
        Self {
            paren_depth: 1,
            ..Self::simple(saved_state)
        }
    }

    fn update_for_command(&mut self, words: &[String]) {
        let Some(command) = words.first().map(String::as_str) else {
            return;
        };

        match command {
            "if" => self.compounds.push(PrescanCompound::If),
            "case" => self.compounds.push(PrescanCompound::Case),
            "for" | "select" | "while" | "until" => {
                self.compounds.push(PrescanCompound::Loop);
            }
            "repeat" if words.iter().any(|word| word == "do") => {
                self.compounds.push(PrescanCompound::Loop);
            }
            "fi" => self.pop_compound(PrescanCompound::If),
            "done" => self.pop_compound(PrescanCompound::Loop),
            "esac" => self.pop_compound(PrescanCompound::Case),
            _ => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.brace_depth == 0 && self.paren_depth == 0 && self.compounds.is_empty()
    }

    fn pop_compound(&mut self, compound: PrescanCompound) {
        if self.compounds.last().copied() == Some(compound) {
            self.compounds.pop();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrescanFunctionHeaderState {
    None,
    AfterWord,
    AfterFunctionKeyword,
    AfterFunctionName,
    AfterWordOpenParen,
    AfterFunctionNameOpenParen,
    ReadyForBrace,
}

impl<'a> ZshOptionPrescanner<'a> {
    fn new(input: &'a str, state: ZshOptionState) -> Self {
        Self {
            input,
            offset: 0,
            state,
            entries: Vec::new(),
        }
    }

    fn scan(mut self) -> Vec<ZshOptionTimelineEntry> {
        let mut words = Vec::new();
        let mut command_end = 0usize;
        let mut local_scopes = Vec::new();
        let mut function_header = PrescanFunctionHeaderState::None;

        while let Some(token) = self.next_token() {
            match token {
                PrescanToken::Word { text, end } => {
                    if is_prescan_function_body_start(function_header) {
                        local_scopes.push(PrescanLocalScope::simple(self.state));
                        function_header = PrescanFunctionHeaderState::None;
                    }
                    command_end = end;
                    function_header = match function_header {
                        PrescanFunctionHeaderState::None => {
                            if text == "function" {
                                PrescanFunctionHeaderState::AfterFunctionKeyword
                            } else {
                                PrescanFunctionHeaderState::AfterWord
                            }
                        }
                        PrescanFunctionHeaderState::AfterFunctionKeyword => {
                            PrescanFunctionHeaderState::AfterFunctionName
                        }
                        _ => PrescanFunctionHeaderState::None,
                    };
                    words.push(text);
                }
                PrescanToken::Separator { kind, start, end } => {
                    self.finish_command(&words, command_end.max(start));
                    if let Some(scope) = local_scopes.last_mut() {
                        scope.update_for_command(&words);
                    }
                    if matches!(
                        kind,
                        PrescanSeparator::Newline | PrescanSeparator::Semicolon
                    ) {
                        self.restore_completed_local_scopes(&mut local_scopes, end);
                    }
                    words.clear();
                    command_end = end;

                    match kind {
                        PrescanSeparator::Newline => {
                            if !matches!(
                                function_header,
                                PrescanFunctionHeaderState::AfterFunctionName
                                    | PrescanFunctionHeaderState::ReadyForBrace
                            ) {
                                function_header = PrescanFunctionHeaderState::None;
                            }
                        }
                        PrescanSeparator::Semicolon
                        | PrescanSeparator::Pipe
                        | PrescanSeparator::Ampersand => {
                            function_header = PrescanFunctionHeaderState::None;
                        }
                        PrescanSeparator::OpenParen => {
                            if is_prescan_function_body_start(function_header) {
                                local_scopes.push(PrescanLocalScope::subshell(self.state));
                                function_header = PrescanFunctionHeaderState::None;
                            } else {
                                let next_header = match function_header {
                                    PrescanFunctionHeaderState::AfterWord => {
                                        PrescanFunctionHeaderState::AfterWordOpenParen
                                    }
                                    PrescanFunctionHeaderState::AfterFunctionName => {
                                        PrescanFunctionHeaderState::AfterFunctionNameOpenParen
                                    }
                                    _ => PrescanFunctionHeaderState::None,
                                };
                                if !matches!(
                                    next_header,
                                    PrescanFunctionHeaderState::AfterWordOpenParen
                                        | PrescanFunctionHeaderState::AfterFunctionNameOpenParen
                                ) {
                                    local_scopes.push(PrescanLocalScope::subshell(self.state));
                                }
                                function_header = next_header;
                            }
                        }
                        PrescanSeparator::CloseParen => {
                            let closes_function_header = matches!(
                                function_header,
                                PrescanFunctionHeaderState::AfterWordOpenParen
                                    | PrescanFunctionHeaderState::AfterFunctionNameOpenParen
                            );
                            function_header = if closes_function_header {
                                PrescanFunctionHeaderState::ReadyForBrace
                            } else {
                                PrescanFunctionHeaderState::None
                            };
                            if !closes_function_header {
                                if let Some(scope) = local_scopes.last_mut()
                                    && scope.paren_depth > 0
                                {
                                    scope.paren_depth -= 1;
                                }
                                self.restore_completed_local_scopes(&mut local_scopes, end);
                            }
                        }
                        PrescanSeparator::OpenBrace => {
                            if is_prescan_function_body_start(function_header) {
                                local_scopes.push(PrescanLocalScope::brace_group(self.state));
                            } else if let Some(scope) = local_scopes.last_mut() {
                                scope.brace_depth += 1;
                            }
                            function_header = PrescanFunctionHeaderState::None;
                        }
                        PrescanSeparator::CloseBrace => {
                            if let Some(scope) = local_scopes.last_mut()
                                && scope.brace_depth > 0
                            {
                                scope.brace_depth -= 1;
                            }
                            self.restore_completed_local_scopes(&mut local_scopes, end);
                            function_header = PrescanFunctionHeaderState::None;
                        }
                    }
                }
            }
        }

        self.finish_command(&words, command_end.max(self.input.len()));
        if let Some(scope) = local_scopes.last_mut() {
            scope.update_for_command(&words);
        }
        self.restore_completed_local_scopes(&mut local_scopes, self.input.len());
        self.entries
    }

    fn finish_command(&mut self, words: &[String], end_offset: usize) {
        let mut next = self.state;
        if !apply_prescan_command_effects(words, &mut next) || next == self.state {
            return;
        }

        self.state = next;
        self.entries.push(ZshOptionTimelineEntry {
            offset: end_offset,
            state: next,
        });
    }

    fn next_token(&mut self) -> Option<PrescanToken> {
        loop {
            self.skip_horizontal_whitespace();
            let ch = self.peek_char()?;

            if ch == '#' && self.state.interactive_comments.is_definitely_on() {
                self.skip_comment();
                continue;
            }

            return match ch {
                '\n' => {
                    let start = self.offset;
                    self.advance_char();
                    Some(PrescanToken::Separator {
                        kind: PrescanSeparator::Newline,
                        start,
                        end: self.offset,
                    })
                }
                ';' | '|' | '&' | '(' | ')' | '{' | '}' => {
                    let start = self.offset;
                    self.advance_char();
                    if matches!(ch, '|' | '&' | ';') && self.peek_char() == Some(ch) {
                        self.advance_char();
                    }
                    let kind = match ch {
                        ';' => PrescanSeparator::Semicolon,
                        '|' => PrescanSeparator::Pipe,
                        '&' => PrescanSeparator::Ampersand,
                        '(' => PrescanSeparator::OpenParen,
                        ')' => PrescanSeparator::CloseParen,
                        '{' => PrescanSeparator::OpenBrace,
                        '}' => PrescanSeparator::CloseBrace,
                        _ => unreachable!(),
                    };
                    Some(PrescanToken::Separator {
                        kind,
                        start,
                        end: self.offset,
                    })
                }
                _ => self
                    .read_word()
                    .map(|(text, end)| PrescanToken::Word { text, end }),
            };
        }
    }

    fn skip_horizontal_whitespace(&mut self) {
        while let Some(ch) = self.peek_char() {
            match ch {
                ' ' | '\t' => {
                    self.advance_char();
                }
                '\\' if self.second_char() == Some('\n') => {
                    self.advance_char();
                    self.advance_char();
                }
                _ => break,
            }
        }
    }

    fn skip_comment(&mut self) {
        while let Some(ch) = self.peek_char() {
            if ch == '\n' {
                break;
            }
            self.advance_char();
        }
    }

    fn read_word(&mut self) -> Option<(String, usize)> {
        let mut text = String::new();

        while let Some(ch) = self.peek_char() {
            if is_prescan_separator(ch) {
                break;
            }

            match ch {
                ' ' | '\t' => break,
                '\\' => {
                    self.advance_char();
                    match self.peek_char() {
                        Some('\n') => {
                            self.advance_char();
                        }
                        Some(next) => {
                            text.push(next);
                            self.advance_char();
                        }
                        None => text.push('\\'),
                    }
                }
                '\'' => {
                    self.advance_char();
                    while let Some(next) = self.peek_char() {
                        if next == '\'' {
                            if self.state.rc_quotes.is_definitely_on()
                                && self.second_char() == Some('\'')
                            {
                                text.push('\'');
                                self.advance_char();
                                self.advance_char();
                                continue;
                            }
                            self.advance_char();
                            break;
                        }
                        text.push(next);
                        self.advance_char();
                    }
                }
                '"' => {
                    self.advance_char();
                    while let Some(next) = self.peek_char() {
                        if next == '"' {
                            self.advance_char();
                            break;
                        }
                        if next == '\\' {
                            self.advance_char();
                            if let Some(escaped) = self.peek_char() {
                                text.push(escaped);
                                self.advance_char();
                            }
                            continue;
                        }
                        text.push(next);
                        self.advance_char();
                    }
                }
                _ => {
                    text.push(ch);
                    self.advance_char();
                }
            }
        }

        (!text.is_empty()).then_some((text, self.offset))
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.offset..].chars().next()
    }

    fn second_char(&self) -> Option<char> {
        let mut chars = self.input[self.offset..].chars();
        chars.next()?;
        chars.next()
    }

    fn advance_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.offset += ch.len_utf8();
        Some(ch)
    }

    fn restore_completed_local_scopes(
        &mut self,
        local_scopes: &mut Vec<PrescanLocalScope>,
        offset: usize,
    ) {
        while local_scopes
            .last()
            .is_some_and(PrescanLocalScope::is_complete)
        {
            let Some(scope) = local_scopes.pop() else {
                unreachable!("scope just matched");
            };
            if self.state != scope.saved_state {
                self.state = scope.saved_state;
                self.entries.push(ZshOptionTimelineEntry {
                    offset,
                    state: scope.saved_state,
                });
            } else {
                self.state = scope.saved_state;
            }
        }
    }
}

fn is_prescan_separator(ch: char) -> bool {
    matches!(ch, '\n' | ';' | '|' | '&' | '(' | ')' | '{' | '}')
}

fn is_prescan_function_body_start(state: PrescanFunctionHeaderState) -> bool {
    matches!(
        state,
        PrescanFunctionHeaderState::AfterFunctionName | PrescanFunctionHeaderState::ReadyForBrace
    )
}

fn apply_prescan_command_effects(words: &[String], state: &mut ZshOptionState) -> bool {
    let Some((command, args_index)) = normalize_prescan_command(words) else {
        return false;
    };

    match command {
        "setopt" => {
            let mut changed = false;
            for arg in &words[args_index..] {
                changed |= state.apply_setopt(arg);
            }
            changed
        }
        "unsetopt" => {
            let mut changed = false;
            for arg in &words[args_index..] {
                changed |= state.apply_unsetopt(arg);
            }
            changed
        }
        "set" => apply_prescan_set_builtin(&words[args_index..], state),
        "emulate" => apply_prescan_emulate(&words[args_index..], state),
        _ => false,
    }
}

fn normalize_prescan_command(words: &[String]) -> Option<(&str, usize)> {
    let mut index = 0usize;

    while let Some(word) = words.get(index) {
        if is_prescan_assignment_word(word) {
            index += 1;
            continue;
        }
        match static_command_wrapper_target_index(words.len(), index, word, |word_index| {
            Some(Cow::Borrowed(words[word_index].as_str()))
        }) {
            StaticCommandWrapperTarget::NotWrapper => {}
            StaticCommandWrapperTarget::Wrapper {
                target_index: Some(target_index),
            } => {
                index = target_index;
                continue;
            }
            StaticCommandWrapperTarget::Wrapper { target_index: None } => return None,
        }
        return Some((word.as_str(), index + 1));
    }

    None
}

fn is_prescan_assignment_word(word: &str) -> bool {
    let Some((name, _value)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn apply_prescan_set_builtin(words: &[String], state: &mut ZshOptionState) -> bool {
    let mut changed = false;
    let mut index = 0usize;

    while let Some(word) = words.get(index) {
        match word.as_str() {
            "-o" | "+o" => {
                let enable = word.starts_with('-');
                if let Some(name) = words.get(index + 1) {
                    changed = if enable {
                        state.apply_setopt(name)
                    } else {
                        state.apply_unsetopt(name)
                    } || changed;
                }
                index += 2;
            }
            _ => {
                if let Some(name) = word.strip_prefix("-o") {
                    changed = state.apply_setopt(name) || changed;
                } else if let Some(name) = word.strip_prefix("+o") {
                    changed = state.apply_unsetopt(name) || changed;
                }
                index += 1;
            }
        }
    }

    changed
}

fn apply_prescan_emulate(words: &[String], state: &mut ZshOptionState) -> bool {
    let mut changed = false;
    let mut mode = None;
    let mut pending_option: Option<bool> = None;
    let mut explicit_updates = Vec::new();
    let mut index = 0usize;

    while let Some(word) = words.get(index) {
        if let Some(enable) = pending_option.take() {
            explicit_updates.push((word.clone(), enable));
            index += 1;
            continue;
        }

        match word.as_str() {
            "-o" | "+o" => {
                pending_option = Some(word.starts_with('-'));
                index += 1;
                continue;
            }
            "zsh" | "sh" | "ksh" | "csh" if mode.is_none() => {
                mode = Some(match word.as_str() {
                    "zsh" => ZshEmulationMode::Zsh,
                    "sh" => ZshEmulationMode::Sh,
                    "ksh" => ZshEmulationMode::Ksh,
                    "csh" => ZshEmulationMode::Csh,
                    _ => unreachable!(),
                });
                index += 1;
                continue;
            }
            _ if mode.is_none() && word.starts_with('-') => {
                for flag in word[1..].chars() {
                    if flag == 'o' {
                        pending_option = Some(true);
                    }
                }
                index += 1;
                continue;
            }
            _ => {}
        }

        index += 1;
    }

    if let Some(mode) = mode {
        *state = ZshOptionState::for_emulate(mode);
        changed = true;
    }

    for (name, enable) in explicit_updates {
        changed = if enable {
            state.apply_setopt(&name)
        } else {
            state.apply_unsetopt(&name)
        } || changed;
    }

    changed
}

#[cfg(test)]
mod tests {
    use shuck_ast::{
        Command as AstCommand, CompoundCommand as AstCompoundCommand, FunctionDef, IfSyntax,
        SimpleCommand as AstSimpleCommand, Stmt,
    };

    use crate::{
        Error,
        parser::{Parser, ShellDialect},
    };

    fn expect_simple(stmt: &Stmt) -> &AstSimpleCommand {
        let AstCommand::Simple(command) = &stmt.command else {
            panic!("expected simple command");
        };
        command
    }

    fn expect_function(stmt: &Stmt) -> &FunctionDef {
        let AstCommand::Function(function) = &stmt.command else {
            panic!("expected function definition");
        };
        function
    }

    fn expect_compound(stmt: &Stmt) -> (&AstCompoundCommand, &[shuck_ast::Redirect]) {
        let AstCommand::Compound(compound) = &stmt.command else {
            panic!("expected compound command");
        };
        (compound, &stmt.redirects)
    }

    #[test]
    fn midfile_unsetopt_short_repeat_demotes_repeat_to_simple_command() {
        let source = "unsetopt short_repeat\nrepeat 2 echo hi\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let command = expect_simple(&output.file.body[1]);

        assert_eq!(command.name.render(source), "repeat");
    }

    #[test]
    fn function_local_unsetopt_short_repeat_does_not_leak_to_top_level() {
        let source = "\
fn() {
  unsetopt short_repeat
  repeat 2 echo local
}
repeat 2 echo global
";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap()
            .file;

        let function = expect_function(&output.body[0]);
        let (compound, _) = expect_compound(function.body.as_ref());
        let AstCompoundCommand::BraceGroup(body) = compound else {
            panic!("expected brace-group function body");
        };
        let local_repeat = expect_simple(&body[1]);
        assert_eq!(local_repeat.name.render(source), "repeat");

        let (compound, _) = expect_compound(&output.body[1]);
        let AstCompoundCommand::Repeat(command) = compound else {
            panic!("expected top-level repeat command");
        };
        assert_eq!(command.count.render(source), "2");
        assert_eq!(command.body.len(), 1);
    }

    #[test]
    fn wrapped_unsetopt_short_repeat_demotes_repeat_to_simple_command() {
        for source in [
            "command unsetopt short_repeat\nrepeat 2 echo hi\n",
            "command -pp unsetopt short_repeat\nrepeat 2 echo hi\n",
            "exec -cl unsetopt short_repeat\nrepeat 2 echo hi\n",
            "exec -lc unsetopt short_repeat\nrepeat 2 echo hi\n",
            "exec -la shuck unsetopt short_repeat\nrepeat 2 echo hi\n",
        ] {
            let output = Parser::with_dialect(source, ShellDialect::Zsh)
                .parse()
                .unwrap()
                .file;

            let command = expect_simple(&output.body[1]);
            assert_eq!(command.name.render(source), "repeat", "{source}");
        }
    }

    #[test]
    fn plain_subshell_does_not_leak_short_repeat_prescan() {
        let source = "( unsetopt short_repeat )\nrepeat 2 echo global\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap()
            .file;

        let (compound, _) = expect_compound(&output.body[0]);
        assert!(matches!(compound, AstCompoundCommand::Subshell(_)));

        let (compound, _) = expect_compound(&output.body[1]);
        let AstCompoundCommand::Repeat(command) = compound else {
            panic!("expected top-level repeat command");
        };
        assert_eq!(command.count.render(source), "2");
    }

    #[test]
    fn command_v_does_not_fake_short_repeat_effects() {
        for source in [
            "command -v unsetopt short_repeat\nrepeat 2 echo global\n",
            "command -pv unsetopt short_repeat\nrepeat 2 echo global\n",
            "command -pV unsetopt short_repeat\nrepeat 2 echo global\n",
        ] {
            let output = Parser::with_dialect(source, ShellDialect::Zsh)
                .parse()
                .unwrap()
                .file;

            let command = expect_simple(&output.body[0]);
            assert_eq!(command.name.render(source), "command", "{source}");

            let (compound, _) = expect_compound(&output.body[1]);
            let AstCompoundCommand::Repeat(repeat) = compound else {
                panic!("expected top-level repeat command for {source}");
            };
            assert_eq!(repeat.count.render(source), "2", "{source}");
        }
    }

    #[test]
    fn unknown_precommand_options_do_not_fake_short_repeat_effects() {
        for source in [
            "command -x unsetopt short_repeat\nrepeat 2 echo global\n",
            "builtin -x unsetopt short_repeat\nrepeat 2 echo global\n",
            "exec -x unsetopt short_repeat\nrepeat 2 echo global\n",
        ] {
            let output = Parser::with_dialect(source, ShellDialect::Zsh)
                .parse()
                .unwrap()
                .file;

            let command = expect_simple(&output.body[0]);
            assert!(
                matches!(
                    command.name.render(source).as_str(),
                    "command" | "builtin" | "exec"
                ),
                "{source}"
            );

            let (compound, _) = expect_compound(&output.body[1]);
            let AstCompoundCommand::Repeat(repeat) = compound else {
                panic!("expected top-level repeat command for {source}");
            };
            assert_eq!(repeat.count.render(source), "2", "{source}");
        }
    }

    #[test]
    fn function_subshell_body_does_not_leak_short_repeat_prescan() {
        let source = "f() ( unsetopt short_repeat )\nrepeat 2 echo global\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap()
            .file;

        let function = expect_function(&output.body[0]);
        let (compound, _) = expect_compound(function.body.as_ref());
        assert!(matches!(compound, AstCompoundCommand::Subshell(_)));

        let (compound, _) = expect_compound(&output.body[1]);
        let AstCompoundCommand::Repeat(command) = compound else {
            panic!("expected top-level repeat command");
        };
        assert_eq!(command.count.render(source), "2");
    }

    #[test]
    fn function_if_body_does_not_leak_short_repeat_prescan() {
        let source = "f() if true; then unsetopt short_repeat; fi\nrepeat 2 echo global\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap()
            .file;

        let function = expect_function(&output.body[0]);
        let (compound, _) = expect_compound(function.body.as_ref());
        assert!(matches!(compound, AstCompoundCommand::If(_)));

        let (compound, _) = expect_compound(&output.body[1]);
        let AstCompoundCommand::Repeat(command) = compound else {
            panic!("expected top-level repeat command");
        };
        assert_eq!(command.count.render(source), "2");
    }

    #[test]
    fn midfile_unsetopt_short_loops_rejects_foreach_loop() {
        Parser::with_dialect("foreach x (a b c) { echo $x; }\n", ShellDialect::Zsh)
            .parse()
            .unwrap();
        let source = "unsetopt short_loops\nforeach x (a b c) { echo $x; }\n";
        let error = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap_err();

        assert!(matches!(
            error,
            Error::Parse { message, .. } if message.contains("foreach loops")
        ));
    }

    #[test]
    fn midfile_setopt_noshortloops_keeps_brace_if_enabled() {
        let source = "setopt noshortloops\nif [[ $profile == ./* || $profile == /* ]] {\n  local localpkg=1\n} elif { ! .zinit-download-file-stdout $URL 0 1 2>/dev/null > $tmpfile } {\n  command rm -f $tmpfile\n}\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();

        let (compound, _) = expect_compound(&output.file.body[1]);
        let AstCompoundCommand::If(command) = compound else {
            panic!("expected if command");
        };
        assert!(matches!(command.syntax, IfSyntax::Brace { .. }));
        assert_eq!(command.elif_branches.len(), 1);
    }

    #[test]
    fn midfile_setopt_ignore_braces_treats_braces_as_words() {
        let source = "setopt ignore_braces\n{ echo hi }\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();

        let command = expect_simple(&output.file.body[1]);
        assert_eq!(command.name.render(source), "{");
        assert_eq!(
            command
                .args
                .iter()
                .map(|word| word.render(source))
                .collect::<Vec<_>>(),
            vec!["echo", "hi", "}"]
        );
    }

    #[test]
    fn midfile_setopt_ignore_braces_disables_brace_syntax_collection() {
        let source = "setopt ignore_braces\nprint {a,b}\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();

        let command = expect_simple(&output.file.body[1]);
        assert!(command.args[0].brace_syntax.is_empty());
    }
}
