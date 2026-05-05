use super::*;

pub(super) fn word_is_semantically_inert(word: &Word) -> bool {
    word.parts
        .iter()
        .all(|part| word_part_is_semantically_inert(&part.kind))
}

pub(super) fn heredoc_body_is_semantically_inert(body: &HeredocBody, source: &str) -> bool {
    body.parts
        .iter()
        .all(|part| heredoc_body_part_is_semantically_inert(&part.kind, part.span, source))
}

pub(super) fn word_part_is_semantically_inert(part: &WordPart) -> bool {
    match part {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } => true,
        WordPart::ZshQualifiedGlob(glob) => zsh_qualified_glob_is_semantically_inert(glob),
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .all(|part| word_part_is_semantically_inert(&part.kind)),
        WordPart::ArithmeticExpansion { expression_ast, .. } => expression_ast.is_none(),
        WordPart::Variable(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::Parameter(_)
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. } => false,
    }
}

pub(super) fn heredoc_body_part_is_semantically_inert(
    part: &HeredocBodyPart,
    span: Span,
    source: &str,
) -> bool {
    match part {
        HeredocBodyPart::Literal(text) => {
            !text.is_source_backed()
                || !escaped_braced_literal_may_contain_reference(text.syntax_str(source, span))
        }
        HeredocBodyPart::ArithmeticExpansion { expression_ast, .. } => expression_ast.is_none(),
        HeredocBodyPart::Variable(_)
        | HeredocBodyPart::CommandSubstitution { .. }
        | HeredocBodyPart::Parameter(_) => false,
    }
}

pub(super) fn zsh_qualified_glob_is_semantically_inert(glob: &shuck_ast::ZshQualifiedGlob) -> bool {
    glob.segments.iter().all(|segment| match segment {
        ZshGlobSegment::Pattern(pattern) => pattern_is_semantically_inert(pattern),
        ZshGlobSegment::InlineControl(_) => true,
    })
}

pub(super) fn pattern_is_semantically_inert(pattern: &Pattern) -> bool {
    pattern
        .parts
        .iter()
        .all(|part| pattern_part_is_semantically_inert(&part.kind))
}

pub(super) fn pattern_part_is_semantically_inert(part: &PatternPart) -> bool {
    match part {
        PatternPart::Literal(_)
        | PatternPart::AnyString
        | PatternPart::AnyChar
        | PatternPart::CharClass(_) => true,
        PatternPart::Group { patterns, .. } => patterns.iter().all(pattern_is_semantically_inert),
        PatternPart::Word(word) => word_is_semantically_inert(word),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_ast::{LiteralText, SourceText};

    fn word(parts: Vec<WordPart>) -> Word {
        let span = Span::new();
        Word {
            parts: parts
                .into_iter()
                .map(|part| WordPartNode::new(part, span))
                .collect(),
            span,
            brace_syntax: Vec::new(),
        }
    }

    fn pattern(parts: Vec<PatternPart>) -> Pattern {
        let span = Span::new();
        Pattern {
            parts: parts
                .into_iter()
                .map(|part| PatternPartNode::new(part, span))
                .collect(),
            span,
        }
    }

    #[test]
    fn inert_word_short_circuits_literal_shapes() {
        let word = word(vec![
            WordPart::Literal(LiteralText::owned("plain")),
            WordPart::DoubleQuoted {
                parts: vec![WordPartNode::new(
                    WordPart::Literal(LiteralText::owned("quoted")),
                    Span::new(),
                )],
                dollar: false,
            },
            WordPart::SingleQuoted {
                value: SourceText::from("single"),
                dollar: false,
            },
        ]);

        assert!(word_is_semantically_inert(&word));
    }

    #[test]
    fn word_with_variable_expansion_is_not_inert() {
        let word = word(vec![
            WordPart::Literal(LiteralText::owned("prefix")),
            WordPart::Variable("HOME".into()),
        ]);

        assert!(!word_is_semantically_inert(&word));
    }

    #[test]
    fn word_with_nested_command_substitution_is_not_inert() {
        let word = word(vec![WordPart::DoubleQuoted {
            parts: vec![WordPartNode::new(
                WordPart::CommandSubstitution {
                    body: StmtSeq {
                        leading_comments: Vec::new(),
                        stmts: Vec::new(),
                        trailing_comments: Vec::new(),
                        span: Span::new(),
                    },
                    syntax: shuck_ast::CommandSubstitutionSyntax::DollarParen,
                },
                Span::new(),
            )],
            dollar: false,
        }]);

        assert!(!word_is_semantically_inert(&word));
    }

    #[test]
    fn inert_zsh_qualified_glob_short_circuits() {
        let word = word(vec![WordPart::ZshQualifiedGlob(
            shuck_ast::ZshQualifiedGlob {
                span: Span::new(),
                segments: vec![
                    ZshGlobSegment::Pattern(pattern(vec![
                        PatternPart::Literal(LiteralText::owned("foo")),
                        PatternPart::AnyString,
                        PatternPart::Group {
                            kind: PatternGroupKind::ExactlyOne,
                            patterns: vec![pattern(vec![PatternPart::CharClass(
                                SourceText::from("[ab]"),
                            )])],
                        },
                    ])),
                    ZshGlobSegment::InlineControl(shuck_ast::ZshInlineGlobControl::StartAnchor {
                        span: Span::new(),
                    }),
                ],
                qualifiers: None,
            },
        )]);

        assert!(word_is_semantically_inert(&word));
    }

    #[test]
    fn pattern_with_expanding_word_is_not_inert() {
        let pattern = pattern(vec![PatternPart::Word(word(vec![
            WordPart::ParameterExpansion {
                reference: VarRef {
                    name: "name".into(),
                    name_span: Span::new(),
                    subscript: None,
                    span: Span::new(),
                },
                operator: Box::new(ParameterOp::UseDefault),
                operand: Some(SourceText::from("fallback")),
                operand_word_ast: Some(Box::new(Word::literal("fallback"))),
                colon_variant: true,
            },
        ]))]);

        assert!(!pattern_is_semantically_inert(&pattern));
    }
}
