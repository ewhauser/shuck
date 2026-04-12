use super::*;
use shuck_ast::{
    AnonymousFunctionCommand as AstAnonymousFunctionCommand, ArithmeticAssignOp,
    ArithmeticBinaryOp, ArithmeticPostfixOp, ArithmeticUnaryOp, BackgroundOperator, BinaryCommand,
    BourneParameterExpansion, BuiltinCommand as AstBuiltinCommand, Command as AstCommand,
    CompoundCommand as AstCompoundCommand, ForSyntax, ForeachSyntax, FunctionDef as AstFunctionDef,
    IfSyntax, Name, ParameterExpansion, ParameterExpansionSyntax, ParameterOp, PrefixMatchKind,
    RepeatSyntax, SimpleCommand as AstSimpleCommand, SourceText, StmtTerminator, ZshDefaultingOp,
    ZshExpansionOperation, ZshExpansionTarget, ZshPatternOp, ZshReplacementOp, ZshTrimOp,
};

fn is_fully_quoted(word: &Word) -> bool {
    word.is_fully_quoted()
}

fn pattern_part_slices<'a>(pattern: &'a Pattern, input: &'a str) -> Vec<&'a str> {
    pattern
        .parts
        .iter()
        .map(|part| part.span.slice(input))
        .collect()
}

fn top_level_part_slices<'a>(word: &'a Word, input: &'a str) -> Vec<&'a str> {
    word.parts
        .iter()
        .map(|part| part.span.slice(input))
        .collect()
}

fn brace_slices<'a>(word: &'a Word, input: &'a str) -> Vec<&'a str> {
    word.brace_syntax
        .iter()
        .map(|brace| brace.span.slice(input))
        .collect()
}

fn redirect_word_target(redirect: &Redirect) -> &Word {
    redirect
        .word_target()
        .expect("expected non-heredoc redirect target")
}

fn redirect_heredoc(redirect: &Redirect) -> &Heredoc {
    redirect.heredoc().expect("expected heredoc redirect")
}

fn collect_file_comments(file: &File) -> Vec<Comment> {
    let mut comments = Vec::new();
    collect_stmt_seq_comments(&file.body, &mut comments);
    comments
}

fn collect_stmt_seq_comments(sequence: &StmtSeq, comments: &mut Vec<Comment>) {
    comments.extend(sequence.leading_comments.iter().copied());
    for stmt in &sequence.stmts {
        collect_stmt_comments(stmt, comments);
    }
    comments.extend(sequence.trailing_comments.iter().copied());
}

fn collect_stmt_comments(stmt: &Stmt, comments: &mut Vec<Comment>) {
    comments.extend(stmt.leading_comments.iter().copied());
    if let Some(comment) = stmt.inline_comment {
        comments.push(comment);
    }
    collect_command_comments(&stmt.command, comments);
}

fn collect_command_comments(command: &AstCommand, comments: &mut Vec<Comment>) {
    match command {
        AstCommand::Binary(command) => {
            collect_stmt_comments(&command.left, comments);
            collect_stmt_comments(&command.right, comments);
        }
        AstCommand::Compound(command) => collect_compound_comments(command, comments),
        AstCommand::Function(function) => collect_stmt_comments(&function.body, comments),
        AstCommand::AnonymousFunction(function) => collect_stmt_comments(&function.body, comments),
        AstCommand::Simple(_) | AstCommand::Builtin(_) | AstCommand::Decl(_) => {}
    }
}

fn collect_compound_comments(command: &AstCompoundCommand, comments: &mut Vec<Comment>) {
    match command {
        AstCompoundCommand::If(command) => {
            collect_stmt_seq_comments(&command.condition, comments);
            collect_stmt_seq_comments(&command.then_branch, comments);
            for branch in &command.elif_branches {
                collect_stmt_seq_comments(&branch.0, comments);
                collect_stmt_seq_comments(&branch.1, comments);
            }
            if let Some(body) = &command.else_branch {
                collect_stmt_seq_comments(body, comments);
            }
        }
        AstCompoundCommand::For(command) => {
            collect_stmt_seq_comments(&command.body, comments);
        }
        AstCompoundCommand::Select(command) => {
            collect_stmt_seq_comments(&command.body, comments);
        }
        AstCompoundCommand::ArithmeticFor(command) => {
            collect_stmt_seq_comments(&command.body, comments);
        }
        AstCompoundCommand::While(command) => {
            collect_stmt_seq_comments(&command.condition, comments);
            collect_stmt_seq_comments(&command.body, comments);
        }
        AstCompoundCommand::Until(command) => {
            collect_stmt_seq_comments(&command.condition, comments);
            collect_stmt_seq_comments(&command.body, comments);
        }
        AstCompoundCommand::Case(command) => {
            for item in &command.cases {
                collect_stmt_seq_comments(&item.body, comments);
            }
        }
        AstCompoundCommand::Subshell(body) | AstCompoundCommand::BraceGroup(body) => {
            collect_stmt_seq_comments(body, comments);
        }
        AstCompoundCommand::Always(command) => {
            collect_stmt_seq_comments(&command.body, comments);
            collect_stmt_seq_comments(&command.always_body, comments);
        }
        AstCompoundCommand::Repeat(command) => {
            collect_stmt_seq_comments(&command.body, comments);
        }
        AstCompoundCommand::Foreach(command) => {
            collect_stmt_seq_comments(&command.body, comments);
        }
        AstCompoundCommand::Conditional(_)
        | AstCompoundCommand::Arithmetic(_)
        | AstCompoundCommand::Time(_)
        | AstCompoundCommand::Coproc(_) => {}
    }
}

fn assert_comment_ranges_valid(source: &str, output: &ParseOutput) {
    let comments = collect_file_comments(&output.file);
    for (i, comment) in comments.iter().enumerate() {
        let start = usize::from(comment.range.start());
        let end = usize::from(comment.range.end());
        assert!(
            end <= source.len(),
            "comment {i}: end ({end}) exceeds source length ({})",
            source.len()
        );
        assert!(
            source.is_char_boundary(start),
            "comment {i}: start ({start}) not on char boundary"
        );
        assert!(
            source.is_char_boundary(end),
            "comment {i}: end ({end}) not on char boundary"
        );
        let text = &source[start..end];
        assert!(
            text.starts_with('#'),
            "comment {i}: expected '#' at start, got {:?}",
            text.chars().next()
        );
        assert!(
            !text.contains('\n'),
            "comment {i}: spans multiple lines: {text:?}"
        );
    }
}

fn expect_function(stmt: &Stmt) -> &AstFunctionDef {
    let AstCommand::Function(function) = &stmt.command else {
        panic!("expected function definition");
    };
    function
}

fn expect_anonymous_function(stmt: &Stmt) -> &AstAnonymousFunctionCommand {
    let AstCommand::AnonymousFunction(function) = &stmt.command else {
        panic!("expected anonymous function");
    };
    function
}

fn expect_compound(stmt: &Stmt) -> (&AstCompoundCommand, &[Redirect]) {
    let AstCommand::Compound(compound) = &stmt.command else {
        panic!("expected compound command");
    };
    (compound, stmt.redirects.as_slice())
}

fn expect_variable(expr: &ArithmeticExprNode, expected: &str) {
    let ArithmeticExpr::Variable(name) = &expr.kind else {
        panic!("expected arithmetic variable, got {:?}", expr.kind);
    };
    assert_eq!(name, expected);
}

fn expect_number(expr: &ArithmeticExprNode, input: &str, expected: &str) {
    let ArithmeticExpr::Number(number) = &expr.kind else {
        panic!("expected arithmetic number, got {:?}", expr.kind);
    };
    assert_eq!(number.slice(input), expected);
}

fn expect_shell_word(expr: &ArithmeticExprNode, input: &str, expected: &str) {
    let ArithmeticExpr::ShellWord(word) = &expr.kind else {
        panic!("expected arithmetic shell word, got {:?}", expr.kind);
    };
    assert_eq!(word.render(input), expected);
}

fn expect_subscript<'a>(reference: &'a VarRef, input: &str, expected: &str) -> &'a Subscript {
    let subscript = reference
        .subscript
        .as_ref()
        .expect("expected subscripted reference");
    assert_eq!(subscript.text.slice(input), expected);
    subscript
}

fn expect_subscript_syntax<'a>(
    reference: &'a VarRef,
    input: &str,
    expected_syntax: &str,
    expected_cooked: &str,
) -> &'a Subscript {
    let subscript = expect_subscript(reference, input, expected_cooked);
    assert_eq!(subscript.syntax_text(input), expected_syntax);
    subscript
}

fn array_access_reference(part: &WordPart) -> Option<&VarRef> {
    match part {
        WordPart::ArrayAccess(reference) => Some(reference),
        WordPart::Parameter(parameter) => match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference }) => {
                Some(reference)
            }
            _ => None,
        },
        _ => None,
    }
}

fn expect_array_access(word: &Word) -> &VarRef {
    let [part] = word.parts.as_slice() else {
        panic!("expected single expansion part");
    };
    array_access_reference(&part.kind)
        .unwrap_or_else(|| panic!("expected array access part, got {:?}", part.kind))
}

fn expect_parameter(word: &Word) -> &ParameterExpansion {
    let [part] = word.parts.as_slice() else {
        panic!("expected single parameter part");
    };
    let WordPart::Parameter(parameter) = &part.kind else {
        panic!("expected parameter part, got {:?}", part.kind);
    };
    parameter
}

fn expect_zsh_qualified_glob(word: &Word) -> &ZshQualifiedGlob {
    let [part] = word.parts.as_slice() else {
        panic!("expected single qualified glob part");
    };
    let WordPart::ZshQualifiedGlob(glob) = &part.kind else {
        panic!("expected qualified glob part, got {:?}", part.kind);
    };
    glob
}

fn expect_zsh_glob_qualifiers(glob: &ZshQualifiedGlob) -> &ZshGlobQualifierGroup {
    glob.qualifiers
        .as_ref()
        .expect("expected zsh glob qualifiers")
}

fn expect_zsh_glob_pattern_segment(segment: &ZshGlobSegment) -> &Pattern {
    let ZshGlobSegment::Pattern(pattern) = segment else {
        panic!("expected pattern segment");
    };
    pattern
}

fn expect_array_length_part(part: &WordPart) -> &VarRef {
    match part {
        WordPart::ArrayLength(reference) => reference,
        WordPart::Parameter(parameter) => match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Length { reference }) => {
                reference
            }
            _ => panic!("expected array length part, got {:?}", part),
        },
        _ => panic!("expected array length part, got {:?}", part),
    }
}

fn expect_array_indices_part(part: &WordPart) -> &VarRef {
    match part {
        WordPart::ArrayIndices(reference) => reference,
        WordPart::Parameter(parameter) => match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Indices { reference }) => {
                reference
            }
            _ => panic!("expected array indices part, got {:?}", part),
        },
        _ => panic!("expected array indices part, got {:?}", part),
    }
}

fn expect_substring_part(
    part: &WordPart,
) -> (
    &VarRef,
    &Option<ArithmeticExprNode>,
    &Option<ArithmeticExprNode>,
) {
    match part {
        WordPart::Substring {
            reference,
            offset_ast,
            length_ast,
            ..
        } => (reference, offset_ast, length_ast),
        WordPart::Parameter(parameter) => match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                length_ast,
                ..
            }) if !reference.has_array_selector() => (reference, offset_ast, length_ast),
            _ => panic!("expected substring part, got {:?}", part),
        },
        _ => panic!("expected substring part, got {:?}", part),
    }
}

fn expect_array_slice_part(
    part: &WordPart,
) -> (
    &VarRef,
    &Option<ArithmeticExprNode>,
    &Option<ArithmeticExprNode>,
) {
    match part {
        WordPart::ArraySlice {
            reference,
            offset_ast,
            length_ast,
            ..
        } => (reference, offset_ast, length_ast),
        WordPart::Parameter(parameter) => match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                length_ast,
                ..
            }) if reference.has_array_selector() => (reference, offset_ast, length_ast),
            _ => panic!("expected array slice part, got {:?}", part),
        },
        _ => panic!("expected array slice part, got {:?}", part),
    }
}

fn expect_parameter_operation_part(
    part: &WordPart,
) -> (&VarRef, &ParameterOp, Option<&SourceText>) {
    match part {
        WordPart::ParameterExpansion {
            reference,
            operator,
            operand,
            ..
        } => (reference, operator, operand.as_ref()),
        WordPart::Parameter(parameter) => match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
                reference,
                operator,
                operand,
                ..
            }) => (reference, operator, operand.as_ref()),
            _ => panic!("expected parameter operation part, got {:?}", part),
        },
        _ => panic!("expected parameter operation part, got {:?}", part),
    }
}

fn expect_prefix_match_part(part: &WordPart) -> (&Name, PrefixMatchKind) {
    match part {
        WordPart::PrefixMatch { prefix, kind } => (prefix, *kind),
        WordPart::Parameter(parameter) => match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::PrefixMatch {
                prefix,
                kind,
            }) => (prefix, *kind),
            _ => panic!("expected prefix match part, got {:?}", part),
        },
        _ => panic!("expected prefix match part, got {:?}", part),
    }
}

fn expect_indirect_expansion_part(
    part: &WordPart,
) -> (&VarRef, Option<&ParameterOp>, Option<&SourceText>, bool) {
    match part {
        WordPart::IndirectExpansion {
            reference,
            operator,
            operand,
            colon_variant,
        } => (
            reference,
            operator.as_ref(),
            operand.as_ref(),
            *colon_variant,
        ),
        WordPart::Parameter(parameter) => match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Indirect {
                reference,
                operator,
                operand,
                colon_variant,
                ..
            }) => (
                reference,
                operator.as_ref(),
                operand.as_ref(),
                *colon_variant,
            ),
            _ => panic!("expected indirect expansion part, got {:?}", part),
        },
        _ => panic!("expected indirect expansion part, got {:?}", part),
    }
}

fn expect_simple(stmt: &Stmt) -> &AstSimpleCommand {
    let AstCommand::Simple(command) = &stmt.command else {
        panic!("expected simple command");
    };
    command
}

fn expect_binary(stmt: &Stmt) -> &BinaryCommand {
    let AstCommand::Binary(command) = &stmt.command else {
        panic!("expected binary command");
    };
    command
}

mod commands;
mod heredocs;
mod redirects;
mod words;
