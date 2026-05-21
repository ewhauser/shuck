use super::*;

pub(crate) fn walk_word_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    word: &mut Word,
) -> usize {
    word.parts
        .iter_mut()
        .map(|part| walk_word_part_surface_source_texts_mut(visitor, part))
        .sum()
}

pub(crate) fn walk_word_part_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    part: &mut WordPartNode,
) -> usize {
    match &mut part.kind {
        WordPart::ZshQualifiedGlob(glob) => {
            walk_zsh_qualified_glob_surface_source_texts_mut(visitor, glob)
        }
        WordPart::SingleQuoted { value, .. } => visitor.visit_source_text(value),
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter_mut()
            .map(|part| walk_word_part_surface_source_texts_mut(visitor, part))
            .sum(),
        WordPart::ArithmeticExpansion { expression, .. } => visitor.visit_source_text(expression),
        WordPart::Parameter(parameter) => visitor.visit_parameter_expansion(parameter),
        WordPart::ParameterExpansion {
            reference,
            operator,
            operand,
            ..
        } => {
            let mut changes = walk_var_ref_surface_source_texts_mut(visitor, reference);
            if let Some(operand) = operand {
                changes += visitor.visit_source_text(operand);
            }
            changes + walk_parameter_op_surface_source_texts_mut(visitor, operator)
        }
        WordPart::Length(reference)
        | WordPart::ArrayAccess(reference)
        | WordPart::ArrayLength(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::Transformation { reference, .. } => {
            walk_var_ref_surface_source_texts_mut(visitor, reference)
        }
        WordPart::Substring {
            reference,
            offset,
            length,
            ..
        }
        | WordPart::ArraySlice {
            reference,
            offset,
            length,
            ..
        } => {
            let mut changes = walk_var_ref_surface_source_texts_mut(visitor, reference)
                + visitor.visit_source_text(offset);
            if let Some(length) = length {
                changes += visitor.visit_source_text(length);
            }
            changes
        }
        WordPart::IndirectExpansion {
            reference,
            operand,
            operator,
            ..
        } => {
            let mut changes = walk_var_ref_surface_source_texts_mut(visitor, reference);
            if let Some(operand) = operand {
                changes += visitor.visit_source_text(operand);
            }
            if let Some(operator) = operator {
                changes += walk_parameter_op_surface_source_texts_mut(visitor, operator);
            }
            changes
        }
        WordPart::Literal(_)
        | WordPart::Variable(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::PrefixMatch { .. } => 0,
    }
}

fn walk_var_ref_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    reference: &mut VarRef,
) -> usize {
    reference.subscript.as_mut().map_or(0, |subscript| {
        walk_subscript_surface_source_texts_mut(visitor, subscript)
    })
}

fn walk_subscript_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    subscript: &mut Subscript,
) -> usize {
    let mut changes = visitor.visit_source_text(&mut subscript.text);
    if let Some(raw) = &mut subscript.raw {
        changes += visitor.visit_source_text(raw);
    }
    changes
}

fn walk_pattern_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    pattern: &mut Pattern,
) -> usize {
    pattern
        .parts
        .iter_mut()
        .map(|part| match &mut part.kind {
            PatternPart::CharClass(text) => visitor.visit_source_text(text),
            PatternPart::Group { patterns, .. } => patterns
                .iter_mut()
                .map(|pattern| walk_pattern_surface_source_texts_mut(visitor, pattern))
                .sum(),
            PatternPart::Word(word) => walk_word_surface_source_texts_mut(visitor, word),
            PatternPart::Literal(_) | PatternPart::AnyString | PatternPart::AnyChar => 0,
        })
        .sum()
}

pub(crate) fn walk_parameter_expansion_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    parameter: &mut ParameterExpansion,
) -> usize {
    match &mut parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                walk_var_ref_surface_source_texts_mut(visitor, reference)
            }
            BourneParameterExpansion::Indirect {
                reference,
                operator,
                operand,
                ..
            } => {
                let mut changes = walk_var_ref_surface_source_texts_mut(visitor, reference);
                if let Some(operand) = operand {
                    changes += visitor.visit_source_text(operand);
                }
                if let Some(operator) = operator {
                    changes += walk_parameter_op_surface_source_texts_mut(visitor, operator);
                }
                changes
            }
            BourneParameterExpansion::PrefixMatch { .. } => 0,
            BourneParameterExpansion::Slice {
                reference,
                offset,
                length,
                ..
            } => {
                let mut changes = walk_var_ref_surface_source_texts_mut(visitor, reference)
                    + visitor.visit_source_text(offset);
                if let Some(length) = length {
                    changes += visitor.visit_source_text(length);
                }
                changes
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand,
                ..
            } => {
                let mut changes = walk_var_ref_surface_source_texts_mut(visitor, reference);
                if let Some(operand) = operand {
                    changes += visitor.visit_source_text(operand);
                }
                changes + walk_parameter_op_surface_source_texts_mut(visitor, operator)
            }
        },
        ParameterExpansionSyntax::Zsh(syntax) => {
            walk_zsh_parameter_expansion_surface_source_texts_mut(visitor, syntax)
        }
    }
}

fn walk_zsh_parameter_expansion_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    syntax: &mut ZshParameterExpansion,
) -> usize {
    match &mut syntax.target {
        ZshExpansionTarget::Reference(reference) => {
            walk_var_ref_surface_source_texts_mut(visitor, reference)
        }
        ZshExpansionTarget::Word(word) => walk_word_surface_source_texts_mut(visitor, word),
        ZshExpansionTarget::Nested(parameter) => visitor.visit_parameter_expansion(parameter),
        ZshExpansionTarget::Empty => 0,
    }
}

fn walk_parameter_op_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    operator: &mut ParameterOp,
) -> usize {
    match operator {
        ParameterOp::RemovePrefixShort { pattern }
        | ParameterOp::RemovePrefixLong { pattern }
        | ParameterOp::RemoveSuffixShort { pattern }
        | ParameterOp::RemoveSuffixLong { pattern } => {
            walk_pattern_surface_source_texts_mut(visitor, pattern)
        }
        ParameterOp::ReplaceFirst {
            pattern,
            replacement,
            ..
        }
        | ParameterOp::ReplaceAll {
            pattern,
            replacement,
            ..
        } => {
            walk_pattern_surface_source_texts_mut(visitor, pattern)
                + visitor.visit_source_text(replacement)
        }
        ParameterOp::UseDefault
        | ParameterOp::AssignDefault
        | ParameterOp::UseReplacement
        | ParameterOp::Error
        | ParameterOp::UpperFirst
        | ParameterOp::UpperAll
        | ParameterOp::LowerFirst
        | ParameterOp::LowerAll => 0,
    }
}

fn walk_zsh_qualified_glob_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    glob: &mut shuck_ast::ZshQualifiedGlob,
) -> usize {
    let mut changes = 0;
    for segment in &mut glob.segments {
        changes += match segment {
            ZshGlobSegment::Pattern(pattern) => {
                walk_pattern_surface_source_texts_mut(visitor, pattern)
            }
            ZshGlobSegment::InlineControl(_) => 0,
        };
    }
    if let Some(qualifiers) = &mut glob.qualifiers {
        changes += walk_zsh_glob_qualifier_group_surface_source_texts_mut(visitor, qualifiers);
    }
    changes
}

fn walk_zsh_glob_qualifier_group_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    group: &mut ZshGlobQualifierGroup,
) -> usize {
    group
        .fragments
        .iter_mut()
        .map(|fragment| match fragment {
            ZshGlobQualifier::LetterSequence { text, .. } => visitor.visit_source_text(text),
            ZshGlobQualifier::NumericArgument { start, end, .. } => {
                let mut changes = visitor.visit_source_text(start);
                if let Some(end) = end {
                    changes += visitor.visit_source_text(end);
                }
                changes
            }
            ZshGlobQualifier::Negation { .. } | ZshGlobQualifier::Flag { .. } => 0,
        })
        .sum()
}
