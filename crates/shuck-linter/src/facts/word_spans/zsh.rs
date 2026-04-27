use super::*;

pub fn word_zsh_flag_modifier_spans(word: &Word) -> Vec<Span> {
    word.parts
        .iter()
        .filter_map(|part| {
            let WordPart::Parameter(parameter) = &part.kind else {
                return None;
            };
            let ParameterExpansionSyntax::Zsh(syntax) = &parameter.syntax else {
                return None;
            };
            if syntax.modifiers.is_empty() {
                return None;
            }
            if syntax
                .modifiers
                .first()
                .is_some_and(|modifier| modifier.name == '=')
            {
                return None;
            }

            match syntax.target {
                ZshExpansionTarget::Reference(_) | ZshExpansionTarget::Word(_) => {}
                ZshExpansionTarget::Nested(_) | ZshExpansionTarget::Empty => return None,
            }

            syntax.modifiers.first().map(|modifier| modifier.span)
        })
        .collect()
}

pub fn word_zsh_nested_expansion_spans(word: &Word) -> Vec<Span> {
    word.parts
        .iter()
        .filter_map(|part| {
            let WordPart::Parameter(parameter) = &part.kind else {
                return None;
            };
            let ParameterExpansionSyntax::Zsh(syntax) = &parameter.syntax else {
                return None;
            };

            matches!(syntax.target, ZshExpansionTarget::Nested(_))
                .then_some(syntax.operation.is_none())
                .filter(|is_none| *is_none)
                .map(|_| parameter.span)
        })
        .collect()
}

pub fn word_nested_zsh_substitution_spans(word: &Word) -> Vec<Span> {
    word.parts
        .iter()
        .filter_map(|part| {
            let WordPart::Parameter(parameter) = &part.kind else {
                return None;
            };
            let ParameterExpansionSyntax::Zsh(syntax) = &parameter.syntax else {
                return None;
            };

            matches!(syntax.target, ZshExpansionTarget::Nested(_))
                .then_some(syntax.operation.as_ref())
                .flatten()
                .map(|_| parameter.span)
        })
        .collect()
}
