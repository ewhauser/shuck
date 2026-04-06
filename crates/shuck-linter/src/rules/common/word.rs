use std::collections::HashMap;

use shuck_ast::{ConditionalExpr, Redirect, RedirectKind, Span, Word, WordPart};

use super::query::{self, CommandSubstitutionKind, CommandWalkOptions, NestedCommandSubstitution};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordQuote {
    Quoted,
    Unquoted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordLiteralness {
    FixedLiteral,
    Expanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordExpansionKind {
    None,
    Scalar,
    Array,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordSubstitutionShape {
    None,
    Plain,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordClassification {
    pub quote: WordQuote,
    pub literalness: WordLiteralness,
    pub expansion_kind: WordExpansionKind,
    pub substitution_shape: WordSubstitutionShape,
}

impl WordClassification {
    pub fn is_fixed_literal(self) -> bool {
        self.literalness == WordLiteralness::FixedLiteral
    }

    pub fn is_expanded(self) -> bool {
        self.literalness == WordLiteralness::Expanded
    }

    pub fn has_scalar_expansion(self) -> bool {
        matches!(
            self.expansion_kind,
            WordExpansionKind::Scalar | WordExpansionKind::Mixed
        )
    }

    pub fn has_array_expansion(self) -> bool {
        matches!(
            self.expansion_kind,
            WordExpansionKind::Array | WordExpansionKind::Mixed
        )
    }

    pub fn has_command_substitution(self) -> bool {
        self.substitution_shape != WordSubstitutionShape::None
    }

    pub fn has_plain_command_substitution(self) -> bool {
        self.substitution_shape == WordSubstitutionShape::Plain
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestOperandClass {
    FixedLiteral,
    RuntimeSensitive,
}

impl TestOperandClass {
    pub fn is_fixed_literal(self) -> bool {
        self == Self::FixedLiteral
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubstitutionOutputIntent {
    Captured,
    Discarded,
    Rerouted,
    Mixed,
}

impl SubstitutionOutputIntent {
    fn merge(self, other: Self) -> Self {
        if self == other { self } else { Self::Mixed }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubstitutionClassification {
    pub kind: CommandSubstitutionKind,
    pub span: Span,
    pub stdout_intent: SubstitutionOutputIntent,
    pub has_stdout_redirect: bool,
}

impl SubstitutionClassification {
    pub fn stdout_is_captured(self) -> bool {
        self.stdout_intent == SubstitutionOutputIntent::Captured
    }

    pub fn stdout_is_discarded(self) -> bool {
        self.stdout_intent == SubstitutionOutputIntent::Discarded
    }

    pub fn stdout_is_rerouted(self) -> bool {
        self.stdout_intent == SubstitutionOutputIntent::Rerouted
    }
}

pub fn static_word_text(word: &Word, source: &str) -> Option<String> {
    let mut result = String::new();
    for (part, span) in word.parts_with_spans() {
        match part {
            WordPart::Literal(text) => result.push_str(text.as_str(source, span)),
            _ => return None,
        }
    }
    Some(result)
}

pub fn classify_word(word: &Word, source: &str) -> WordClassification {
    let mut has_non_literal = false;
    let mut has_scalar_expansion = false;
    let mut has_array_expansion = false;
    let mut command_substitution_count = 0usize;

    for part in &word.parts {
        match part {
            WordPart::Literal(_) => {}
            WordPart::CommandSubstitution(_) => {
                has_non_literal = true;
                command_substitution_count += 1;
            }
            WordPart::ProcessSubstitution { .. } => has_non_literal = true,
            WordPart::Variable(_)
            | WordPart::ArithmeticExpansion(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::Substring { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::Transformation { .. } => {
                has_non_literal = true;
                has_scalar_expansion = true;
            }
            WordPart::ArrayAccess { index, .. } => {
                has_non_literal = true;
                if matches!(index.slice(source), "@" | "*") {
                    has_array_expansion = true;
                } else {
                    has_scalar_expansion = true;
                }
            }
            WordPart::ArraySlice { .. } => {
                has_non_literal = true;
                has_array_expansion = true;
            }
            WordPart::ArrayLength(_) | WordPart::PrefixMatch(_) => {
                has_non_literal = true;
                has_scalar_expansion = true;
            }
            WordPart::ArrayIndices(_) => {
                has_non_literal = true;
                has_array_expansion = true;
            }
        }
    }

    WordClassification {
        quote: if word.quoted {
            WordQuote::Quoted
        } else {
            WordQuote::Unquoted
        },
        literalness: if has_non_literal {
            WordLiteralness::Expanded
        } else {
            WordLiteralness::FixedLiteral
        },
        expansion_kind: match (has_scalar_expansion, has_array_expansion) {
            (false, false) => WordExpansionKind::None,
            (true, false) => WordExpansionKind::Scalar,
            (false, true) => WordExpansionKind::Array,
            (true, true) => WordExpansionKind::Mixed,
        },
        substitution_shape: if command_substitution_count == 0 {
            WordSubstitutionShape::None
        } else if matches!(word.parts.as_slice(), [WordPart::CommandSubstitution(_)]) {
            WordSubstitutionShape::Plain
        } else {
            WordSubstitutionShape::Mixed
        },
    }
}

pub fn classify_test_operand(word: &Word, source: &str) -> TestOperandClass {
    if static_word_text(word, source).is_some() {
        TestOperandClass::FixedLiteral
    } else {
        TestOperandClass::RuntimeSensitive
    }
}

pub fn classify_conditional_operand(
    expression: &ConditionalExpr,
    source: &str,
) -> TestOperandClass {
    match expression {
        ConditionalExpr::Word(word)
        | ConditionalExpr::Pattern(word)
        | ConditionalExpr::Regex(word) => classify_test_operand(word, source),
        ConditionalExpr::Parenthesized(expression) => {
            classify_conditional_operand(&expression.expr, source)
        }
        ConditionalExpr::Binary(_) | ConditionalExpr::Unary(_) => {
            TestOperandClass::RuntimeSensitive
        }
    }
}

pub fn classify_substitution(
    substitution: NestedCommandSubstitution<'_>,
    source: &str,
) -> SubstitutionClassification {
    let mut stdout_intent: Option<SubstitutionOutputIntent> = None;
    let mut has_stdout_redirect = false;

    query::walk_commands(
        substitution.commands,
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
        &mut |command, _| {
            let state = classify_command_redirects(query::command_redirects(command), source);
            has_stdout_redirect |= state.has_stdout_redirect;
            stdout_intent = Some(match stdout_intent {
                Some(current) => current.merge(state.stdout_intent),
                None => state.stdout_intent,
            });
        },
    );

    SubstitutionClassification {
        kind: substitution.kind,
        span: substitution.span,
        stdout_intent: stdout_intent.unwrap_or(SubstitutionOutputIntent::Captured),
        has_stdout_redirect,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputSink {
    Captured,
    DevNull,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RedirectState {
    stdout_intent: SubstitutionOutputIntent,
    has_stdout_redirect: bool,
}

fn classify_command_redirects(redirects: &[Redirect], source: &str) -> RedirectState {
    let mut fds = HashMap::from([(1, OutputSink::Captured), (2, OutputSink::Other)]);
    let mut has_stdout_redirect = false;

    for redirect in redirects {
        match redirect.kind {
            RedirectKind::Output | RedirectKind::Clobber | RedirectKind::Append => {
                let sink = redirect_file_sink(redirect, source);
                let fd = redirect.fd.unwrap_or(1);
                has_stdout_redirect |= fd == 1;
                fds.insert(fd, sink);
            }
            RedirectKind::OutputBoth => {
                let sink = redirect_file_sink(redirect, source);
                has_stdout_redirect = true;
                fds.insert(1, sink);
                fds.insert(2, sink);
            }
            RedirectKind::DupOutput => {
                let fd = redirect.fd.unwrap_or(1);
                let sink = redirect_dup_output_sink(redirect, &fds, source);
                has_stdout_redirect |= fd == 1;
                fds.insert(fd, sink);
            }
            RedirectKind::Input
            | RedirectKind::ReadWrite
            | RedirectKind::HereDoc
            | RedirectKind::HereDocStrip
            | RedirectKind::HereString
            | RedirectKind::DupInput => {}
        }
    }

    let stdout_sink = *fds.get(&1).unwrap_or(&OutputSink::Other);
    let stderr_sink = *fds.get(&2).unwrap_or(&OutputSink::Other);
    let stdout_intent = if matches!(stdout_sink, OutputSink::Captured)
        || matches!(stderr_sink, OutputSink::Captured)
    {
        SubstitutionOutputIntent::Captured
    } else if matches!(stdout_sink, OutputSink::DevNull) {
        SubstitutionOutputIntent::Discarded
    } else {
        SubstitutionOutputIntent::Rerouted
    };

    RedirectState {
        stdout_intent,
        has_stdout_redirect,
    }
}

fn redirect_file_sink(redirect: &Redirect, source: &str) -> OutputSink {
    if static_word_text(&redirect.target, source).as_deref() == Some("/dev/null") {
        OutputSink::DevNull
    } else {
        OutputSink::Other
    }
}

fn redirect_dup_output_sink(
    redirect: &Redirect,
    fds: &HashMap<i32, OutputSink>,
    source: &str,
) -> OutputSink {
    let Some(target) = static_word_text(&redirect.target, source) else {
        return OutputSink::Other;
    };

    let Ok(fd) = target.parse::<i32>() else {
        return OutputSink::Other;
    };

    *fds.get(&fd).unwrap_or(&OutputSink::Other)
}

#[cfg(test)]
mod tests {
    use shuck_ast::{Command, CompoundCommand};
    use shuck_parser::parser::Parser;

    use super::{
        SubstitutionOutputIntent, TestOperandClass, WordExpansionKind, WordLiteralness, WordQuote,
        WordSubstitutionShape, classify_conditional_operand, classify_substitution,
        classify_test_operand, classify_word,
    };
    use crate::rules::common::query::iter_word_command_substitutions;

    fn parse_commands(source: &str) -> Vec<Command> {
        Parser::new(source).parse().unwrap().script.commands
    }

    #[test]
    fn classify_word_distinguishes_fixed_literals_and_quoted_expansions() {
        let source = "printf \"literal\" \"prefix$foo\"\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0] else {
            panic!("expected simple command");
        };

        let literal = classify_word(&command.args[0], source);
        assert_eq!(literal.quote, WordQuote::Quoted);
        assert_eq!(literal.literalness, WordLiteralness::FixedLiteral);
        assert_eq!(literal.expansion_kind, WordExpansionKind::None);
        assert_eq!(literal.substitution_shape, WordSubstitutionShape::None);

        let expanded = classify_word(&command.args[1], source);
        assert_eq!(expanded.quote, WordQuote::Quoted);
        assert_eq!(expanded.literalness, WordLiteralness::Expanded);
        assert_eq!(expanded.expansion_kind, WordExpansionKind::Scalar);
        assert_eq!(expanded.substitution_shape, WordSubstitutionShape::None);
    }

    #[test]
    fn classify_word_reports_plain_and_mixed_command_substitutions() {
        let source = "printf \"$(date)\" \"prefix$(date)\"\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0] else {
            panic!("expected simple command");
        };

        assert_eq!(
            classify_word(&command.args[0], source).substitution_shape,
            WordSubstitutionShape::Plain
        );
        assert_eq!(
            classify_word(&command.args[1], source).substitution_shape,
            WordSubstitutionShape::Mixed
        );
    }

    #[test]
    fn classify_word_reports_scalar_and_array_expansions() {
        let source = "printf $foo ${arr[@]} ${arr[0]} ${arr[@]:1}\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0] else {
            panic!("expected simple command");
        };

        assert_eq!(
            classify_word(&command.args[0], source).expansion_kind,
            WordExpansionKind::Scalar
        );
        assert_eq!(
            classify_word(&command.args[1], source).expansion_kind,
            WordExpansionKind::Array
        );
        assert_eq!(
            classify_word(&command.args[2], source).expansion_kind,
            WordExpansionKind::Scalar
        );
        assert_eq!(
            classify_word(&command.args[3], source).expansion_kind,
            WordExpansionKind::Array
        );
    }

    #[test]
    fn classify_test_and_conditional_operands_share_literal_runtime_decisions() {
        let source = "test foo\n[[ \"$re\" ]]\n[[ literal ]]\n";
        let commands = parse_commands(source);

        let Command::Simple(simple_test) = &commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(
            classify_test_operand(&simple_test.args[0], source),
            TestOperandClass::FixedLiteral
        );

        let Command::Compound(CompoundCommand::Conditional(runtime), _) = &commands[1] else {
            panic!("expected conditional");
        };
        assert_eq!(
            classify_conditional_operand(&runtime.expression, source),
            TestOperandClass::RuntimeSensitive
        );

        let Command::Compound(CompoundCommand::Conditional(literal), _) = &commands[2] else {
            panic!("expected conditional");
        };
        assert_eq!(
            classify_conditional_operand(&literal.expression, source),
            TestOperandClass::FixedLiteral
        );
    }

    #[test]
    fn classify_substitution_reports_stdout_intent_and_redirects() {
        let cases = [
            (
                "out=$(printf hi)\n",
                SubstitutionOutputIntent::Captured,
                false,
            ),
            (
                "out=$(printf hi > out.txt)\n",
                SubstitutionOutputIntent::Rerouted,
                true,
            ),
            (
                "out=$(printf hi >/dev/null 2>&1)\n",
                SubstitutionOutputIntent::Discarded,
                true,
            ),
            (
                "out=$(whiptail 3>&1 1>&2 2>&3)\n",
                SubstitutionOutputIntent::Captured,
                true,
            ),
            (
                "out=$(jq -r . <<< \"$status\" || die >&2)\n",
                SubstitutionOutputIntent::Mixed,
                true,
            ),
            (
                "out=$(awk 'BEGIN { print \"ok\" }' || warn >&2)\n",
                SubstitutionOutputIntent::Mixed,
                true,
            ),
            (
                "out=$(getopt -o a -- \"$@\" || { usage >&2 && false; })\n",
                SubstitutionOutputIntent::Mixed,
                true,
            ),
            (
                "out=$(\"${cmd[@]}\" \"${options[@]}\" 2>&1 >/dev/tty)\n",
                SubstitutionOutputIntent::Captured,
                true,
            ),
            (
                "out=$(cat <<'EOF'\nhello\nEOF\n)\n",
                SubstitutionOutputIntent::Captured,
                false,
            ),
            (
                "out=$(printf quiet >/dev/null; printf loud)\n",
                SubstitutionOutputIntent::Mixed,
                true,
            ),
            (
                "out=$(printf quiet >/dev/null; printf loud > out.txt)\n",
                SubstitutionOutputIntent::Mixed,
                true,
            ),
        ];

        for (source, expected_intent, expected_redirect) in cases {
            let commands = parse_commands(source);
            let Command::Simple(command) = &commands[0] else {
                panic!("expected simple command");
            };
            let substitution =
                iter_word_command_substitutions(match &command.assignments[0].value {
                    shuck_ast::AssignmentValue::Scalar(word) => word,
                    shuck_ast::AssignmentValue::Array(_) => panic!("expected scalar assignment"),
                })
                .next()
                .expect("expected command substitution");

            let classification = classify_substitution(substitution, source);
            assert_eq!(classification.stdout_intent, expected_intent);
            assert_eq!(classification.has_stdout_redirect, expected_redirect);
        }
    }
}
