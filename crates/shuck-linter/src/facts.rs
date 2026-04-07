use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    AssignmentValue, BuiltinCommand, Command, CompoundCommand, DeclOperand, File, Redirect, Span,
    Stmt, Word,
};

use crate::rules::common::{
    command::{self, NormalizedCommand},
    query::{self, CommandVisit, CommandWalkOptions},
    word::static_word_text,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FactSpan {
    start: usize,
    end: usize,
}

impl FactSpan {
    pub fn new(span: Span) -> Self {
        Self {
            start: span.start.offset,
            end: span.end.offset,
        }
    }
}

impl From<Span> for FactSpan {
    fn from(span: Span) -> Self {
        Self::new(span)
    }
}

#[derive(Debug, Clone)]
pub struct CommandFact<'a> {
    key: FactSpan,
    visit: CommandVisit<'a>,
    nested_word_command: bool,
    normalized: NormalizedCommand<'a>,
    options: CommandOptionFacts<'a>,
}

impl<'a> CommandFact<'a> {
    pub fn key(&self) -> FactSpan {
        self.key
    }

    pub fn visit(&self) -> CommandVisit<'a> {
        self.visit
    }

    pub fn is_nested_word_command(&self) -> bool {
        self.nested_word_command
    }

    pub fn stmt(&self) -> &'a Stmt {
        self.visit.stmt
    }

    pub fn command(&self) -> &'a Command {
        self.visit.command
    }

    pub fn redirects(&self) -> &'a [Redirect] {
        self.visit.redirects
    }

    pub fn normalized(&self) -> &NormalizedCommand<'a> {
        &self.normalized
    }

    pub fn literal_name(&self) -> Option<&str> {
        self.normalized.literal_name.as_deref()
    }

    pub fn effective_name_is(&self, name: &str) -> bool {
        self.normalized.effective_name_is(name)
    }

    pub fn declaration(&self) -> Option<&command::NormalizedDeclaration<'a>> {
        self.normalized.declaration.as_ref()
    }

    pub fn body_name_word(&self) -> Option<&'a Word> {
        self.normalized.body_name_word()
    }

    pub fn body_args(&self) -> &[&'a Word] {
        self.normalized.body_args()
    }

    pub fn read_uses_raw_input(&self) -> Option<bool> {
        self.options.read.as_ref().map(|read| read.uses_raw_input)
    }

    pub fn printf_format_word(&self) -> Option<&'a Word> {
        self.options
            .printf
            .as_ref()
            .and_then(|printf| printf.format_word)
    }
}

#[derive(Debug, Clone, Default)]
struct CommandOptionFacts<'a> {
    read: Option<ReadCommandFacts>,
    printf: Option<PrintfCommandFacts<'a>>,
}

impl<'a> CommandOptionFacts<'a> {
    fn build(normalized: &NormalizedCommand<'a>, source: &str) -> Self {
        Self {
            read: normalized
                .effective_name_is("read")
                .then(|| ReadCommandFacts {
                    uses_raw_input: read_uses_raw_input(normalized.body_args(), source),
                }),
            printf: normalized
                .effective_name_is("printf")
                .then(|| PrintfCommandFacts {
                    format_word: printf_format_word(normalized.body_args(), source),
                }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ReadCommandFacts {
    uses_raw_input: bool,
}

#[derive(Debug, Clone, Copy)]
struct PrintfCommandFacts<'a> {
    format_word: Option<&'a Word>,
}

#[derive(Debug, Clone)]
pub struct LinterFacts<'a> {
    commands: Vec<CommandFact<'a>>,
    command_index: FxHashMap<FactSpan, usize>,
    scalar_bindings: FxHashMap<FactSpan, &'a Word>,
}

impl<'a> LinterFacts<'a> {
    pub fn build(file: &'a File, source: &'a str) -> Self {
        let structural_commands = query::iter_commands(
            &file.body,
            CommandWalkOptions {
                descend_nested_word_commands: false,
            },
        )
        .map(|visit| FactSpan::new(command_span(visit.command)))
        .collect::<FxHashSet<_>>();
        let mut commands = Vec::new();
        let mut command_index = FxHashMap::default();
        let mut scalar_bindings = FxHashMap::default();

        for visit in query::iter_commands(
            &file.body,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
        ) {
            let key = FactSpan::new(command_span(visit.command));
            let previous = command_index.insert(key, commands.len());
            debug_assert!(previous.is_none(), "duplicate command fact key: {key:?}");

            collect_scalar_bindings(visit.command, &mut scalar_bindings);
            let normalized = command::normalize_command(visit.command, source);
            commands.push(CommandFact {
                key,
                visit,
                nested_word_command: !structural_commands.contains(&key),
                options: CommandOptionFacts::build(&normalized, source),
                normalized,
            });
        }

        Self {
            commands,
            command_index,
            scalar_bindings,
        }
    }

    pub fn commands(&self) -> &[CommandFact<'a>] {
        &self.commands
    }

    pub fn command(&self, span: Span) -> Option<&CommandFact<'a>> {
        self.command_index
            .get(&FactSpan::new(span))
            .map(|&index| &self.commands[index])
    }

    pub fn scalar_binding_value(&self, span: Span) -> Option<&'a Word> {
        self.scalar_bindings.get(&FactSpan::new(span)).copied()
    }

    pub(crate) fn scalar_binding_values(&self) -> &FxHashMap<FactSpan, &'a Word> {
        &self.scalar_bindings
    }
}

fn read_uses_raw_input(args: &[&Word], source: &str) -> bool {
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        let mut chars = text[1..].chars().peekable();
        while let Some(flag) = chars.next() {
            if flag == 'r' {
                return true;
            }

            if option_takes_argument(flag) {
                if chars.peek().is_none() {
                    index += 1;
                }
                break;
            }
        }

        index += 1;
    }

    false
}

fn option_takes_argument(flag: char) -> bool {
    matches!(flag, 'a' | 'd' | 'i' | 'n' | 'N' | 'p' | 't' | 'u')
}

fn printf_format_word<'a>(args: &[&'a Word], source: &str) -> Option<&'a Word> {
    let mut index = 0usize;

    if static_word_text(args.get(index)?, source).as_deref() == Some("--") {
        index += 1;
    }

    if let Some(option) = args
        .get(index)
        .and_then(|word| static_word_text(word, source))
    {
        if option == "-v" {
            index += 2;
        } else if option.starts_with("-v") && option.len() > 2 {
            index += 1;
        }
    }

    args.get(index).copied()
}

fn collect_scalar_bindings<'a>(
    command: &'a Command,
    scalar_bindings: &mut FxHashMap<FactSpan, &'a Word>,
) {
    for assignment in query::command_assignments(command) {
        let AssignmentValue::Scalar(word) = &assignment.value else {
            continue;
        };
        scalar_bindings.insert(FactSpan::new(assignment.target.name_span), word);
    }

    for operand in query::declaration_operands(command) {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };
        let AssignmentValue::Scalar(word) = &assignment.value else {
            continue;
        };
        scalar_bindings.insert(FactSpan::new(assignment.target.name_span), word);
    }
}

fn command_span(command: &Command) -> Span {
    match command {
        Command::Simple(command) => command.span,
        Command::Builtin(command) => builtin_span(command),
        Command::Decl(command) => command.span,
        Command::Binary(command) => command.span,
        Command::Compound(command) => compound_span(command),
        Command::Function(command) => command.span,
    }
}

fn builtin_span(command: &BuiltinCommand) -> Span {
    match command {
        BuiltinCommand::Break(command) => command.span,
        BuiltinCommand::Continue(command) => command.span,
        BuiltinCommand::Return(command) => command.span,
        BuiltinCommand::Exit(command) => command.span,
    }
}

fn compound_span(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::If(command) => command.span,
        CompoundCommand::For(command) => command.span,
        CompoundCommand::ArithmeticFor(command) => command.span,
        CompoundCommand::While(command) => command.span,
        CompoundCommand::Until(command) => command.span,
        CompoundCommand::Case(command) => command.span,
        CompoundCommand::Select(command) => command.span,
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            commands.span
        }
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command.span,
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command.span,
    }
}

#[cfg(test)]
mod tests {
    use shuck_ast::{Command, DeclOperand};
    use shuck_parser::parser::Parser;

    use super::LinterFacts;
    use crate::rules::common::command::WrapperKind;

    #[test]
    fn builds_command_facts_for_wrapped_and_nested_commands() {
        let source = "#!/bin/bash\ncommand printf '%s\\n' \"$(echo hi)\"\n";
        let output = Parser::new(source).parse().unwrap();
        let facts = LinterFacts::build(&output.file, source);

        assert_eq!(facts.commands().len(), 2);

        let Command::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };
        let outer = facts
            .command(command.span)
            .expect("expected fact for outer command");

        assert_eq!(outer.normalized().effective_name.as_deref(), Some("printf"));
        assert_eq!(outer.normalized().wrappers, vec![WrapperKind::Command]);
        assert!(!outer.is_nested_word_command());
        assert_eq!(
            outer
                .printf_format_word()
                .map(|word| word.span.slice(source)),
            Some("'%s\\n'")
        );
        assert_eq!(
            facts.commands()[1].normalized().effective_name.as_deref(),
            Some("echo")
        );
        assert!(facts.commands()[1].is_nested_word_command());
    }

    #[test]
    fn indexes_scalar_bindings_from_assignments_and_declarations() {
        let source = "#!/bin/bash\nfoo=1 printf '%s\\n' \"$foo\"\nexport bar=2\n";
        let output = Parser::new(source).parse().unwrap();
        let facts = LinterFacts::build(&output.file, source);

        let first_binding_span = match &output.file.body[0].command {
            Command::Simple(command) => command.assignments[0].target.name_span,
            _ => panic!("expected simple command"),
        };
        assert_eq!(
            facts
                .scalar_binding_value(first_binding_span)
                .map(|word| word.span.slice(source)),
            Some("1")
        );

        let second_binding_span = match &output.file.body[1].command {
            Command::Decl(command) => match &command.operands[0] {
                DeclOperand::Assignment(assignment) => assignment.target.name_span,
                _ => panic!("expected declaration assignment"),
            },
            _ => panic!("expected declaration command"),
        };
        assert_eq!(
            facts
                .scalar_binding_value(second_binding_span)
                .map(|word| word.span.slice(source)),
            Some("2")
        );
    }

    #[test]
    fn summarizes_read_and_printf_options() {
        let source = "#!/bin/bash\nread -r name\nprintf -v out \"$fmt\" value\n";
        let output = Parser::new(source).parse().unwrap();
        let facts = LinterFacts::build(&output.file, source);

        let read = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("read"))
            .expect("expected read fact");
        assert_eq!(read.read_uses_raw_input(), Some(true));

        let printf = facts
            .commands()
            .iter()
            .find(|fact| fact.effective_name_is("printf"))
            .expect("expected printf fact");
        assert_eq!(
            printf
                .printf_format_word()
                .map(|word| word.span.slice(source)),
            Some("\"$fmt\"")
        );
    }
}
