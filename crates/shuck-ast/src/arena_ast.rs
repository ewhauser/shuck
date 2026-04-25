use crate::{
    AnonymousFunctionCommand, ArithmeticCommand, ArithmeticExpr, ArithmeticExprNode,
    ArithmeticForCommand, ArithmeticLvalue, Assignment, AssignmentValue, BinaryCommand,
    BuiltinCommand, CaseCommand, Command, Comment, CompoundCommand, ConditionalCommand,
    ConditionalExpr, DeclOperand, File, ForCommand, FunctionDef, HeredocBodyPart, IdRange, Idx,
    ListArena, Pattern, PatternPart, Redirect, RedirectTarget, RepeatCommand, SelectCommand, Span,
    Stmt, StmtSeq, StmtTerminator, Subscript, Word, WordPart, WordPartNode,
};

/// Stable typed identifier for a parsed file node inside an [`AstStore`].
pub type FileId = Idx<FileNode>;
/// Stable typed identifier for a statement sequence node inside an [`AstStore`].
pub type StmtSeqId = Idx<StmtSeqNode>;
/// Stable typed identifier for a statement node inside an [`AstStore`].
pub type StmtId = Idx<StmtNode>;
/// Stable typed identifier for a command node inside an [`AstStore`].
pub type CommandId = Idx<CommandNode>;
/// Stable typed identifier for a word node inside an [`AstStore`].
pub type WordId = Idx<WordNode>;

/// An ID-backed parsed file representation.
#[derive(Debug, Clone)]
pub struct ArenaFile {
    /// Root file node.
    pub root: FileId,
    /// Arena storage for the parsed file.
    pub store: AstStore,
}

impl ArenaFile {
    /// Builds an arena representation from the existing recursive AST.
    pub fn from_file(file: &File) -> Self {
        let mut builder = AstStoreBuilder::default();
        let root = builder.lower_file(file);
        Self {
            root,
            store: builder.finish(),
        }
    }

    /// Builds an arena representation from an owned parsed root body.
    pub fn from_body(body: StmtSeq, span: Span) -> Self {
        let mut builder = AstStoreBuilder::default();
        let root = builder.lower_file_body(body, span);
        Self {
            root,
            store: builder.finish(),
        }
    }

    /// Returns a borrowed view of the root file node.
    pub fn view(&self) -> FileView<'_> {
        self.store.file(self.root)
    }

    /// Materializes the arena representation back into the legacy recursive AST.
    pub fn to_file(&self) -> File {
        self.store.materialize_file(self.root)
    }
}

/// Compact typed AST storage for one parsed file.
#[derive(Debug, Clone)]
pub struct AstStore {
    files: Vec<FileNode>,
    stmt_seqs: Vec<StmtSeqNode>,
    stmts: Vec<StmtNode>,
    commands: Vec<CommandNode>,
    words: Vec<WordNode>,
    stmt_id_lists: ListArena<StmtId>,
    stmt_seq_id_lists: ListArena<StmtSeqId>,
    comment_lists: ListArena<Comment>,
    redirect_lists: ListArena<RedirectNode>,
    assignment_lists: ListArena<AssignmentNode>,
    array_elem_lists: ListArena<ArrayElemNode>,
    decl_operand_lists: ListArena<DeclOperandNode>,
    heredoc_body_part_lists: ListArena<ArenaHeredocBodyPartNode>,
    zsh_modifier_lists: ListArena<ZshModifierNode>,
    function_header_entry_lists: ListArena<FunctionHeaderEntryNode>,
    elif_branch_lists: ListArena<ElifBranchNode>,
    for_target_lists: ListArena<ForTargetNode>,
    case_item_lists: ListArena<CaseItemNode>,
    pattern_lists: ListArena<PatternNode>,
    pattern_part_lists: ListArena<PatternPartArenaNode>,
    zsh_glob_segment_lists: ListArena<ZshGlobSegmentNode>,
    zsh_glob_qualifier_lists: ListArena<crate::ZshGlobQualifier>,
    word_id_lists: ListArena<WordId>,
    word_part_lists: ListArena<WordPartArenaNode>,
    brace_syntax_lists: ListArena<crate::BraceSyntax>,
}

impl Default for AstStore {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            stmt_seqs: Vec::new(),
            stmts: Vec::new(),
            commands: Vec::new(),
            words: Vec::new(),
            stmt_id_lists: ListArena::new(),
            stmt_seq_id_lists: ListArena::new(),
            comment_lists: ListArena::new(),
            redirect_lists: ListArena::new(),
            assignment_lists: ListArena::new(),
            array_elem_lists: ListArena::new(),
            decl_operand_lists: ListArena::new(),
            heredoc_body_part_lists: ListArena::new(),
            zsh_modifier_lists: ListArena::new(),
            function_header_entry_lists: ListArena::new(),
            elif_branch_lists: ListArena::new(),
            for_target_lists: ListArena::new(),
            case_item_lists: ListArena::new(),
            pattern_lists: ListArena::new(),
            pattern_part_lists: ListArena::new(),
            zsh_glob_segment_lists: ListArena::new(),
            zsh_glob_qualifier_lists: ListArena::new(),
            word_id_lists: ListArena::new(),
            word_part_lists: ListArena::new(),
            brace_syntax_lists: ListArena::new(),
        }
    }
}

impl AstStore {
    /// Returns the file view for `id`.
    pub fn file(&self, id: FileId) -> FileView<'_> {
        FileView { store: self, id }
    }

    /// Returns the statement sequence view for `id`.
    pub fn stmt_seq(&self, id: StmtSeqId) -> StmtSeqView<'_> {
        StmtSeqView { store: self, id }
    }

    /// Returns the statement view for `id`.
    pub fn stmt(&self, id: StmtId) -> StmtView<'_> {
        StmtView { store: self, id }
    }

    /// Returns the command view for `id`.
    pub fn command(&self, id: CommandId) -> CommandView<'_> {
        CommandView { store: self, id }
    }

    /// Returns the word view for `id`.
    pub fn word(&self, id: WordId) -> WordView<'_> {
        WordView { store: self, id }
    }

    /// Returns word IDs from a list range.
    pub fn word_ids(&self, range: IdRange<WordId>) -> &[WordId] {
        self.word_id_lists.get(range)
    }

    /// Returns array element nodes from a list range.
    pub fn array_elems(&self, range: IdRange<ArrayElemNode>) -> &[ArrayElemNode] {
        self.array_elem_lists.get(range)
    }

    /// Returns heredoc body part nodes from a list range.
    pub fn heredoc_body_parts(
        &self,
        range: IdRange<ArenaHeredocBodyPartNode>,
    ) -> &[ArenaHeredocBodyPartNode] {
        self.heredoc_body_part_lists.get(range)
    }

    /// Returns elif branch nodes from a list range.
    pub fn elif_branches(&self, range: IdRange<ElifBranchNode>) -> &[ElifBranchNode] {
        self.elif_branch_lists.get(range)
    }

    /// Returns case item nodes from a list range.
    pub fn case_items(&self, range: IdRange<CaseItemNode>) -> &[CaseItemNode] {
        self.case_item_lists.get(range)
    }

    /// Returns pattern nodes from a list range.
    pub fn patterns(&self, range: IdRange<PatternNode>) -> &[PatternNode] {
        self.pattern_lists.get(range)
    }

    /// Returns pattern part nodes from a list range.
    pub fn pattern_parts(&self, range: IdRange<PatternPartArenaNode>) -> &[PatternPartArenaNode] {
        self.pattern_part_lists.get(range)
    }

    /// Returns zsh glob segment nodes from a list range.
    pub fn zsh_glob_segments(&self, range: IdRange<ZshGlobSegmentNode>) -> &[ZshGlobSegmentNode] {
        self.zsh_glob_segment_lists.get(range)
    }

    /// Returns word part nodes from a list range.
    pub fn word_parts(&self, range: IdRange<WordPartArenaNode>) -> &[WordPartArenaNode] {
        self.word_part_lists.get(range)
    }

    /// Number of file nodes in this store.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Number of statement sequence nodes in this store.
    pub fn stmt_seq_count(&self) -> usize {
        self.stmt_seqs.len()
    }

    /// Number of statement nodes in this store.
    pub fn stmt_count(&self) -> usize {
        self.stmts.len()
    }

    /// Number of command nodes in this store.
    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    /// Number of word nodes in this store.
    pub fn word_count(&self) -> usize {
        self.words.len()
    }

    fn materialize_file(&self, id: FileId) -> File {
        let node = &self.files[id.index()];
        File {
            body: self.materialize_stmt_seq(node.body),
            span: node.span,
        }
    }

    fn materialize_stmt_seq(&self, id: StmtSeqId) -> StmtSeq {
        let node = &self.stmt_seqs[id.index()];
        StmtSeq {
            leading_comments: self.comment_lists.get(node.leading_comments).to_vec(),
            stmts: self
                .stmt_id_lists
                .get(node.stmts)
                .iter()
                .copied()
                .map(|stmt| self.materialize_stmt(stmt))
                .collect(),
            trailing_comments: self.comment_lists.get(node.trailing_comments).to_vec(),
            span: node.span,
        }
    }

    fn materialize_stmt(&self, id: StmtId) -> Stmt {
        let node = &self.stmts[id.index()];
        Stmt {
            leading_comments: self.comment_lists.get(node.leading_comments).to_vec(),
            command: self.materialize_command(node.command),
            negated: node.negated,
            redirects: self
                .redirect_lists
                .get(node.redirects)
                .iter()
                .map(|redirect| self.materialize_redirect(redirect))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            terminator: node.terminator,
            terminator_span: node.terminator_span,
            inline_comment: node.inline_comment,
            span: node.span,
        }
    }

    fn materialize_redirect(&self, node: &RedirectNode) -> Redirect {
        Redirect {
            fd: node.fd,
            fd_var: node.fd_var.clone(),
            fd_var_span: node.fd_var_span,
            kind: node.kind,
            span: node.span,
            target: match &node.target {
                RedirectTargetNode::Word(word) => {
                    RedirectTarget::Word(self.materialize_word(*word))
                }
                RedirectTargetNode::Heredoc(heredoc) => RedirectTarget::Heredoc(crate::Heredoc {
                    delimiter: crate::HeredocDelimiter {
                        raw: self.materialize_word(heredoc.delimiter.raw),
                        cooked: heredoc.delimiter.cooked.clone(),
                        span: heredoc.delimiter.span,
                        quoted: heredoc.delimiter.quoted,
                        expands_body: heredoc.delimiter.expands_body,
                        strip_tabs: heredoc.delimiter.strip_tabs,
                    },
                    body: crate::HeredocBody {
                        mode: heredoc.body.mode,
                        source_backed: heredoc.body.source_backed,
                        parts: self
                            .heredoc_body_part_lists
                            .get(heredoc.body.parts)
                            .iter()
                            .map(|part| self.materialize_heredoc_body_part(part))
                            .collect(),
                        span: heredoc.body.span,
                    },
                }),
            },
        }
    }

    fn materialize_heredoc_body_part(
        &self,
        node: &ArenaHeredocBodyPartNode,
    ) -> crate::HeredocBodyPartNode {
        crate::HeredocBodyPartNode {
            kind: match &node.kind {
                ArenaHeredocBodyPart::Literal(text) => HeredocBodyPart::Literal(text.clone()),
                ArenaHeredocBodyPart::Variable(name) => HeredocBodyPart::Variable(name.clone()),
                ArenaHeredocBodyPart::CommandSubstitution { body, syntax } => {
                    HeredocBodyPart::CommandSubstitution {
                        body: self.materialize_stmt_seq(*body),
                        syntax: *syntax,
                    }
                }
                ArenaHeredocBodyPart::ArithmeticExpansion {
                    expression,
                    expression_ast,
                    expression_word_ast,
                    syntax,
                } => HeredocBodyPart::ArithmeticExpansion {
                    expression: expression.clone(),
                    expression_ast: expression_ast
                        .as_ref()
                        .map(|expr| self.materialize_arithmetic_expr(expr)),
                    expression_word_ast: self.materialize_word(*expression_word_ast),
                    syntax: *syntax,
                },
                ArenaHeredocBodyPart::Parameter(expansion) => HeredocBodyPart::Parameter(Box::new(
                    self.materialize_parameter_expansion(expansion),
                )),
            },
            span: node.span,
        }
    }

    fn materialize_assignment_list(&self, range: IdRange<AssignmentNode>) -> Vec<Assignment> {
        self.assignment_lists
            .get(range)
            .iter()
            .map(|assignment| self.materialize_assignment(assignment))
            .collect()
    }

    fn materialize_decl_operand(&self, node: &DeclOperandNode) -> DeclOperand {
        match node {
            DeclOperandNode::Flag(word) => DeclOperand::Flag(self.materialize_word(*word)),
            DeclOperandNode::Name(reference) => {
                DeclOperand::Name(self.materialize_var_ref(reference))
            }
            DeclOperandNode::Assignment(assignment) => {
                DeclOperand::Assignment(self.materialize_assignment(assignment))
            }
            DeclOperandNode::Dynamic(word) => DeclOperand::Dynamic(self.materialize_word(*word)),
        }
    }

    fn materialize_assignment(&self, node: &AssignmentNode) -> Assignment {
        Assignment {
            target: self.materialize_var_ref(&node.target),
            value: match &node.value {
                AssignmentValueNode::Scalar(word) => {
                    AssignmentValue::Scalar(self.materialize_word(*word))
                }
                AssignmentValueNode::Compound(array) => {
                    AssignmentValue::Compound(self.materialize_array_expr(array))
                }
            },
            append: node.append,
            span: node.span,
        }
    }

    fn materialize_var_ref(&self, node: &VarRefNode) -> crate::VarRef {
        crate::VarRef {
            name: node.name.clone(),
            name_span: node.name_span,
            subscript: node
                .subscript
                .as_deref()
                .map(|subscript| Box::new(self.materialize_subscript(subscript))),
            span: node.span,
        }
    }

    fn materialize_subscript(&self, node: &SubscriptNode) -> Subscript {
        Subscript {
            text: node.text.clone(),
            raw: node.raw.clone(),
            kind: node.kind,
            interpretation: node.interpretation,
            word_ast: node.word_ast.map(|word| self.materialize_word(word)),
            arithmetic_ast: node
                .arithmetic_ast
                .as_ref()
                .map(|expr| self.materialize_arithmetic_expr(expr)),
        }
    }

    fn materialize_array_expr(&self, node: &ArrayExprNode) -> crate::ArrayExpr {
        crate::ArrayExpr {
            kind: node.kind,
            elements: self
                .array_elem_lists
                .get(node.elements)
                .iter()
                .map(|element| self.materialize_array_elem(element))
                .collect(),
            span: node.span,
        }
    }

    fn materialize_array_elem(&self, node: &ArrayElemNode) -> crate::ArrayElem {
        match node {
            ArrayElemNode::Sequential(value) => {
                crate::ArrayElem::Sequential(self.materialize_array_value_word(value))
            }
            ArrayElemNode::Keyed { key, value } => crate::ArrayElem::Keyed {
                key: self.materialize_subscript(key),
                value: self.materialize_array_value_word(value),
            },
            ArrayElemNode::KeyedAppend { key, value } => crate::ArrayElem::KeyedAppend {
                key: self.materialize_subscript(key),
                value: self.materialize_array_value_word(value),
            },
        }
    }

    fn materialize_array_value_word(&self, node: &ArrayValueWordNode) -> crate::ArrayValueWord {
        crate::ArrayValueWord::new(
            self.materialize_word(node.word),
            node.has_top_level_unquoted_comma,
        )
    }

    fn materialize_pattern(&self, node: &PatternNode) -> Pattern {
        Pattern {
            parts: self
                .pattern_part_lists
                .get(node.parts)
                .iter()
                .map(|part| self.materialize_pattern_part(part))
                .collect(),
            span: node.span,
        }
    }

    fn materialize_pattern_part(&self, node: &PatternPartArenaNode) -> crate::PatternPartNode {
        crate::PatternPartNode {
            kind: match &node.kind {
                PatternPartArena::Literal(text) => PatternPart::Literal(text.clone()),
                PatternPartArena::AnyString => PatternPart::AnyString,
                PatternPartArena::AnyChar => PatternPart::AnyChar,
                PatternPartArena::CharClass(text) => PatternPart::CharClass(text.clone()),
                PatternPartArena::Group { kind, patterns } => PatternPart::Group {
                    kind: *kind,
                    patterns: self
                        .pattern_lists
                        .get(*patterns)
                        .iter()
                        .map(|pattern| self.materialize_pattern(pattern))
                        .collect(),
                },
                PatternPartArena::Word(word) => PatternPart::Word(self.materialize_word(*word)),
            },
            span: node.span,
        }
    }

    fn materialize_zsh_qualified_glob(
        &self,
        node: &ZshQualifiedGlobNode,
    ) -> crate::ZshQualifiedGlob {
        crate::ZshQualifiedGlob {
            span: node.span,
            segments: self
                .zsh_glob_segment_lists
                .get(node.segments)
                .iter()
                .map(|segment| match segment {
                    ZshGlobSegmentNode::Pattern(pattern) => {
                        crate::ZshGlobSegment::Pattern(self.materialize_pattern(pattern))
                    }
                    ZshGlobSegmentNode::InlineControl(control) => {
                        crate::ZshGlobSegment::InlineControl(*control)
                    }
                })
                .collect(),
            qualifiers: node
                .qualifiers
                .as_ref()
                .map(|qualifiers| crate::ZshGlobQualifierGroup {
                    span: qualifiers.span,
                    kind: qualifiers.kind,
                    fragments: self
                        .zsh_glob_qualifier_lists
                        .get(qualifiers.fragments)
                        .to_vec(),
                }),
        }
    }

    fn materialize_parameter_expansion(
        &self,
        node: &ParameterExpansionNode,
    ) -> crate::ParameterExpansion {
        crate::ParameterExpansion {
            syntax: match &node.syntax {
                ParameterExpansionSyntaxNode::Bourne(expansion) => {
                    crate::ParameterExpansionSyntax::Bourne(
                        self.materialize_bourne_parameter_expansion(expansion),
                    )
                }
                ParameterExpansionSyntaxNode::Zsh(expansion) => {
                    crate::ParameterExpansionSyntax::Zsh(
                        self.materialize_zsh_parameter_expansion(expansion),
                    )
                }
            },
            span: node.span,
            raw_body: node.raw_body.clone(),
        }
    }

    fn materialize_bourne_parameter_expansion(
        &self,
        node: &BourneParameterExpansionNode,
    ) -> crate::BourneParameterExpansion {
        match node {
            BourneParameterExpansionNode::Access { reference } => {
                crate::BourneParameterExpansion::Access {
                    reference: self.materialize_var_ref(reference),
                }
            }
            BourneParameterExpansionNode::Length { reference } => {
                crate::BourneParameterExpansion::Length {
                    reference: self.materialize_var_ref(reference),
                }
            }
            BourneParameterExpansionNode::Indices { reference } => {
                crate::BourneParameterExpansion::Indices {
                    reference: self.materialize_var_ref(reference),
                }
            }
            BourneParameterExpansionNode::Indirect {
                reference,
                operator,
                operand,
                operand_word_ast,
                colon_variant,
            } => crate::BourneParameterExpansion::Indirect {
                reference: self.materialize_var_ref(reference),
                operator: operator.clone(),
                operand: operand.clone(),
                operand_word_ast: operand_word_ast.map(|word| self.materialize_word(word)),
                colon_variant: *colon_variant,
            },
            BourneParameterExpansionNode::PrefixMatch { prefix, kind } => {
                crate::BourneParameterExpansion::PrefixMatch {
                    prefix: prefix.clone(),
                    kind: *kind,
                }
            }
            BourneParameterExpansionNode::Slice {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
            } => crate::BourneParameterExpansion::Slice {
                reference: self.materialize_var_ref(reference),
                offset: offset.clone(),
                offset_ast: offset_ast
                    .as_ref()
                    .map(|expr| self.materialize_arithmetic_expr(expr)),
                offset_word_ast: self.materialize_word(*offset_word_ast),
                length: length.clone(),
                length_ast: length_ast
                    .as_ref()
                    .map(|expr| self.materialize_arithmetic_expr(expr)),
                length_word_ast: length_word_ast.map(|word| self.materialize_word(word)),
            },
            BourneParameterExpansionNode::Operation {
                reference,
                operator,
                operand,
                operand_word_ast,
                colon_variant,
            } => crate::BourneParameterExpansion::Operation {
                reference: self.materialize_var_ref(reference),
                operator: operator.clone(),
                operand: operand.clone(),
                operand_word_ast: operand_word_ast.map(|word| self.materialize_word(word)),
                colon_variant: *colon_variant,
            },
            BourneParameterExpansionNode::Transformation {
                reference,
                operator,
            } => crate::BourneParameterExpansion::Transformation {
                reference: self.materialize_var_ref(reference),
                operator: *operator,
            },
        }
    }

    fn materialize_zsh_parameter_expansion(
        &self,
        node: &ZshParameterExpansionNode,
    ) -> crate::ZshParameterExpansion {
        crate::ZshParameterExpansion {
            target: self.materialize_zsh_expansion_target(&node.target),
            modifiers: self
                .zsh_modifier_lists
                .get(node.modifiers)
                .iter()
                .map(|modifier| self.materialize_zsh_modifier(modifier))
                .collect(),
            length_prefix: node.length_prefix,
            operation: node
                .operation
                .as_ref()
                .map(|operation| self.materialize_zsh_expansion_operation(operation)),
        }
    }

    fn materialize_zsh_expansion_target(
        &self,
        node: &ZshExpansionTargetNode,
    ) -> crate::ZshExpansionTarget {
        match node {
            ZshExpansionTargetNode::Reference(reference) => {
                crate::ZshExpansionTarget::Reference(self.materialize_var_ref(reference))
            }
            ZshExpansionTargetNode::Nested(expansion) => crate::ZshExpansionTarget::Nested(
                Box::new(self.materialize_parameter_expansion(expansion)),
            ),
            ZshExpansionTargetNode::Word(word) => {
                crate::ZshExpansionTarget::Word(self.materialize_word(*word))
            }
            ZshExpansionTargetNode::Empty => crate::ZshExpansionTarget::Empty,
        }
    }

    fn materialize_zsh_modifier(&self, node: &ZshModifierNode) -> crate::ZshModifier {
        crate::ZshModifier {
            name: node.name,
            argument: node.argument.clone(),
            argument_word_ast: node
                .argument_word_ast
                .map(|word| self.materialize_word(word)),
            argument_delimiter: node.argument_delimiter,
            span: node.span,
        }
    }

    fn materialize_zsh_expansion_operation(
        &self,
        node: &ZshExpansionOperationNode,
    ) -> crate::ZshExpansionOperation {
        match node {
            ZshExpansionOperationNode::PatternOperation {
                kind,
                operand,
                operand_word_ast,
            } => crate::ZshExpansionOperation::PatternOperation {
                kind: *kind,
                operand: operand.clone(),
                operand_word_ast: self.materialize_word(*operand_word_ast),
            },
            ZshExpansionOperationNode::Defaulting {
                kind,
                operand,
                operand_word_ast,
                colon_variant,
            } => crate::ZshExpansionOperation::Defaulting {
                kind: *kind,
                operand: operand.clone(),
                operand_word_ast: self.materialize_word(*operand_word_ast),
                colon_variant: *colon_variant,
            },
            ZshExpansionOperationNode::TrimOperation {
                kind,
                operand,
                operand_word_ast,
            } => crate::ZshExpansionOperation::TrimOperation {
                kind: *kind,
                operand: operand.clone(),
                operand_word_ast: self.materialize_word(*operand_word_ast),
            },
            ZshExpansionOperationNode::ReplacementOperation {
                kind,
                pattern,
                pattern_word_ast,
                replacement,
                replacement_word_ast,
            } => crate::ZshExpansionOperation::ReplacementOperation {
                kind: *kind,
                pattern: pattern.clone(),
                pattern_word_ast: self.materialize_word(*pattern_word_ast),
                replacement: replacement.clone(),
                replacement_word_ast: replacement_word_ast.map(|word| self.materialize_word(word)),
            },
            ZshExpansionOperationNode::Slice {
                offset,
                offset_word_ast,
                length,
                length_word_ast,
            } => crate::ZshExpansionOperation::Slice {
                offset: offset.clone(),
                offset_word_ast: self.materialize_word(*offset_word_ast),
                length: length.clone(),
                length_word_ast: length_word_ast.map(|word| self.materialize_word(word)),
            },
            ZshExpansionOperationNode::Unknown { text, word_ast } => {
                crate::ZshExpansionOperation::Unknown {
                    text: text.clone(),
                    word_ast: self.materialize_word(*word_ast),
                }
            }
        }
    }

    fn materialize_conditional_expr(&self, node: &ConditionalExprArena) -> ConditionalExpr {
        match node {
            ConditionalExprArena::Binary {
                left,
                op,
                op_span,
                right,
            } => ConditionalExpr::Binary(crate::ConditionalBinaryExpr {
                left: Box::new(self.materialize_conditional_expr(left)),
                op: *op,
                op_span: *op_span,
                right: Box::new(self.materialize_conditional_expr(right)),
            }),
            ConditionalExprArena::Unary { op, op_span, expr } => {
                ConditionalExpr::Unary(crate::ConditionalUnaryExpr {
                    op: *op,
                    op_span: *op_span,
                    expr: Box::new(self.materialize_conditional_expr(expr)),
                })
            }
            ConditionalExprArena::Parenthesized {
                left_paren_span,
                expr,
                right_paren_span,
            } => ConditionalExpr::Parenthesized(crate::ConditionalParenExpr {
                left_paren_span: *left_paren_span,
                expr: Box::new(self.materialize_conditional_expr(expr)),
                right_paren_span: *right_paren_span,
            }),
            ConditionalExprArena::Word(word) => ConditionalExpr::Word(self.materialize_word(*word)),
            ConditionalExprArena::Pattern(pattern) => {
                ConditionalExpr::Pattern(self.materialize_pattern(pattern))
            }
            ConditionalExprArena::Regex(word) => {
                ConditionalExpr::Regex(self.materialize_word(*word))
            }
            ConditionalExprArena::VarRef(reference) => {
                ConditionalExpr::VarRef(Box::new(self.materialize_var_ref(reference)))
            }
        }
    }

    fn materialize_arithmetic_expr(&self, node: &ArithmeticExprArenaNode) -> ArithmeticExprNode {
        ArithmeticExprNode {
            kind: match &node.kind {
                ArithmeticExprArena::Number(text) => ArithmeticExpr::Number(text.clone()),
                ArithmeticExprArena::Variable(name) => ArithmeticExpr::Variable(name.clone()),
                ArithmeticExprArena::Indexed { name, index } => ArithmeticExpr::Indexed {
                    name: name.clone(),
                    index: Box::new(self.materialize_arithmetic_expr(index)),
                },
                ArithmeticExprArena::ShellWord(word) => {
                    ArithmeticExpr::ShellWord(self.materialize_word(*word))
                }
                ArithmeticExprArena::Parenthesized { expression } => {
                    ArithmeticExpr::Parenthesized {
                        expression: Box::new(self.materialize_arithmetic_expr(expression)),
                    }
                }
                ArithmeticExprArena::Unary { op, expr } => ArithmeticExpr::Unary {
                    op: *op,
                    expr: Box::new(self.materialize_arithmetic_expr(expr)),
                },
                ArithmeticExprArena::Postfix { expr, op } => ArithmeticExpr::Postfix {
                    expr: Box::new(self.materialize_arithmetic_expr(expr)),
                    op: *op,
                },
                ArithmeticExprArena::Binary { left, op, right } => ArithmeticExpr::Binary {
                    left: Box::new(self.materialize_arithmetic_expr(left)),
                    op: *op,
                    right: Box::new(self.materialize_arithmetic_expr(right)),
                },
                ArithmeticExprArena::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => ArithmeticExpr::Conditional {
                    condition: Box::new(self.materialize_arithmetic_expr(condition)),
                    then_expr: Box::new(self.materialize_arithmetic_expr(then_expr)),
                    else_expr: Box::new(self.materialize_arithmetic_expr(else_expr)),
                },
                ArithmeticExprArena::Assignment { target, op, value } => {
                    ArithmeticExpr::Assignment {
                        target: self.materialize_arithmetic_lvalue(target),
                        op: *op,
                        value: Box::new(self.materialize_arithmetic_expr(value)),
                    }
                }
            },
            span: node.span,
        }
    }

    fn materialize_arithmetic_lvalue(&self, node: &ArithmeticLvalueArena) -> ArithmeticLvalue {
        match node {
            ArithmeticLvalueArena::Variable(name) => ArithmeticLvalue::Variable(name.clone()),
            ArithmeticLvalueArena::Indexed { name, index } => ArithmeticLvalue::Indexed {
                name: name.clone(),
                index: Box::new(self.materialize_arithmetic_expr(index)),
            },
        }
    }

    fn materialize_command(&self, id: CommandId) -> Command {
        let node = &self.commands[id.index()];
        match &node.payload {
            CommandNodePayload::Simple(command) => Command::Simple(crate::SimpleCommand {
                name: self.materialize_word(command.name),
                args: self
                    .word_id_lists
                    .get(command.args)
                    .iter()
                    .copied()
                    .map(|id| self.materialize_word(id))
                    .collect(),
                assignments: self
                    .materialize_assignment_list(command.assignments)
                    .into_boxed_slice(),
                span: node.span,
            }),
            CommandNodePayload::Builtin(command) => {
                let primary = command.primary.map(|id| self.materialize_word(id));
                let extra_args = self
                    .word_id_lists
                    .get(command.extra_args)
                    .iter()
                    .copied()
                    .map(|id| self.materialize_word(id))
                    .collect();
                let assignments = self
                    .materialize_assignment_list(command.assignments)
                    .into_boxed_slice();
                let command = match command.kind {
                    BuiltinCommandNodeKind::Break => BuiltinCommand::Break(crate::BreakCommand {
                        depth: primary,
                        extra_args,
                        assignments,
                        span: node.span,
                    }),
                    BuiltinCommandNodeKind::Continue => {
                        BuiltinCommand::Continue(crate::ContinueCommand {
                            depth: primary,
                            extra_args,
                            assignments,
                            span: node.span,
                        })
                    }
                    BuiltinCommandNodeKind::Return => {
                        BuiltinCommand::Return(crate::ReturnCommand {
                            code: primary,
                            extra_args,
                            assignments,
                            span: node.span,
                        })
                    }
                    BuiltinCommandNodeKind::Exit => BuiltinCommand::Exit(crate::ExitCommand {
                        code: primary,
                        extra_args,
                        assignments,
                        span: node.span,
                    }),
                };
                Command::Builtin(command)
            }
            CommandNodePayload::Decl(command) => Command::Decl(crate::DeclClause {
                variant: command.variant.clone(),
                variant_span: command.variant_span,
                operands: self
                    .decl_operand_lists
                    .get(command.operands)
                    .iter()
                    .map(|operand| self.materialize_decl_operand(operand))
                    .collect(),
                assignments: self
                    .materialize_assignment_list(command.assignments)
                    .into_boxed_slice(),
                span: node.span,
            }),
            CommandNodePayload::Binary(command) => {
                let mut left_stmts = self.materialize_stmt_seq(command.left).stmts.into_iter();
                let Some(left) = left_stmts.next() else {
                    panic!("binary left sequence contains one statement");
                };
                let mut right_stmts = self.materialize_stmt_seq(command.right).stmts.into_iter();
                let Some(right) = right_stmts.next() else {
                    panic!("binary right sequence contains one statement");
                };
                Command::Binary(crate::BinaryCommand {
                    left: Box::new(left),
                    op: command.op,
                    op_span: command.op_span,
                    right: Box::new(right),
                    span: node.span,
                })
            }
            CommandNodePayload::Function(command) => {
                let mut body = self.materialize_stmt_seq(command.body).stmts.into_iter();
                let Some(body) = body.next() else {
                    panic!("function body sequence contains one statement");
                };
                Command::Function(crate::FunctionDef {
                    header: crate::FunctionHeader {
                        function_keyword_span: command.function_keyword_span,
                        entries: self
                            .function_header_entry_lists
                            .get(command.entries)
                            .iter()
                            .map(|entry| crate::FunctionHeaderEntry {
                                word: self.materialize_word(entry.word),
                                static_name: entry.static_name.clone(),
                            })
                            .collect(),
                        trailing_parens_span: command.trailing_parens_span,
                    },
                    body: Box::new(body),
                    span: node.span,
                })
            }
            CommandNodePayload::AnonymousFunction(command) => {
                let mut body = self.materialize_stmt_seq(command.body).stmts.into_iter();
                let Some(body) = body.next() else {
                    panic!("anonymous function body sequence contains one statement");
                };
                Command::AnonymousFunction(crate::AnonymousFunctionCommand {
                    surface: command.surface,
                    body: Box::new(body),
                    args: self
                        .word_id_lists
                        .get(command.args)
                        .iter()
                        .copied()
                        .map(|id| self.materialize_word(id))
                        .collect(),
                    span: node.span,
                })
            }
            CommandNodePayload::Compound(command) => {
                Command::Compound(self.materialize_compound_command(command, node.span))
            }
        }
    }

    fn materialize_compound_command(
        &self,
        command: &CompoundCommandNode,
        span: Span,
    ) -> CompoundCommand {
        match command {
            CompoundCommandNode::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
                syntax,
            } => CompoundCommand::If(crate::IfCommand {
                condition: self.materialize_stmt_seq(*condition),
                then_branch: self.materialize_stmt_seq(*then_branch),
                elif_branches: self
                    .elif_branch_lists
                    .get(*elif_branches)
                    .iter()
                    .map(|branch| {
                        (
                            self.materialize_stmt_seq(branch.condition),
                            self.materialize_stmt_seq(branch.body),
                        )
                    })
                    .collect(),
                else_branch: else_branch.map(|id| self.materialize_stmt_seq(id)),
                syntax: *syntax,
                span,
            }),
            CompoundCommandNode::For {
                targets,
                words,
                body,
                syntax,
            } => CompoundCommand::For(crate::ForCommand {
                targets: self
                    .for_target_lists
                    .get(*targets)
                    .iter()
                    .map(|target| crate::ForTarget {
                        word: self.materialize_word(target.word),
                        name: target.name.clone(),
                        span: target.span,
                    })
                    .collect(),
                words: words.map(|range| {
                    self.word_id_lists
                        .get(range)
                        .iter()
                        .copied()
                        .map(|id| self.materialize_word(id))
                        .collect()
                }),
                body: self.materialize_stmt_seq(*body),
                syntax: *syntax,
                span,
            }),
            CompoundCommandNode::Repeat {
                count,
                body,
                syntax,
            } => CompoundCommand::Repeat(crate::RepeatCommand {
                count: self.materialize_word(*count),
                body: self.materialize_stmt_seq(*body),
                syntax: *syntax,
                span,
            }),
            CompoundCommandNode::Foreach {
                variable,
                variable_span,
                words,
                body,
                syntax,
            } => CompoundCommand::Foreach(crate::ForeachCommand {
                variable: variable.clone(),
                variable_span: *variable_span,
                words: self
                    .word_id_lists
                    .get(*words)
                    .iter()
                    .copied()
                    .map(|id| self.materialize_word(id))
                    .collect(),
                body: self.materialize_stmt_seq(*body),
                syntax: *syntax,
                span,
            }),
            CompoundCommandNode::ArithmeticFor(command) => {
                CompoundCommand::ArithmeticFor(Box::new(crate::ArithmeticForCommand {
                    left_paren_span: command.left_paren_span,
                    init_span: command.init_span,
                    init_ast: command
                        .init_ast
                        .as_ref()
                        .map(|expr| self.materialize_arithmetic_expr(expr)),
                    first_semicolon_span: command.first_semicolon_span,
                    condition_span: command.condition_span,
                    condition_ast: command
                        .condition_ast
                        .as_ref()
                        .map(|expr| self.materialize_arithmetic_expr(expr)),
                    second_semicolon_span: command.second_semicolon_span,
                    step_span: command.step_span,
                    step_ast: command
                        .step_ast
                        .as_ref()
                        .map(|expr| self.materialize_arithmetic_expr(expr)),
                    right_paren_span: command.right_paren_span,
                    body: self.materialize_stmt_seq(command.body),
                    span,
                }))
            }
            CompoundCommandNode::While { condition, body } => {
                CompoundCommand::While(crate::WhileCommand {
                    condition: self.materialize_stmt_seq(*condition),
                    body: self.materialize_stmt_seq(*body),
                    span,
                })
            }
            CompoundCommandNode::Until { condition, body } => {
                CompoundCommand::Until(crate::UntilCommand {
                    condition: self.materialize_stmt_seq(*condition),
                    body: self.materialize_stmt_seq(*body),
                    span,
                })
            }
            CompoundCommandNode::Case { word, cases } => {
                CompoundCommand::Case(crate::CaseCommand {
                    word: self.materialize_word(*word),
                    cases: self
                        .case_item_lists
                        .get(*cases)
                        .iter()
                        .map(|case| crate::CaseItem {
                            patterns: self
                                .pattern_lists
                                .get(case.patterns)
                                .iter()
                                .map(|pattern| self.materialize_pattern(pattern))
                                .collect(),
                            body: self.materialize_stmt_seq(case.body),
                            terminator: case.terminator,
                            terminator_span: case.terminator_span,
                        })
                        .collect(),
                    span,
                })
            }
            CompoundCommandNode::Select {
                variable,
                variable_span,
                words,
                body,
            } => CompoundCommand::Select(crate::SelectCommand {
                variable: variable.clone(),
                variable_span: *variable_span,
                words: self
                    .word_id_lists
                    .get(*words)
                    .iter()
                    .copied()
                    .map(|id| self.materialize_word(id))
                    .collect(),
                body: self.materialize_stmt_seq(*body),
                span,
            }),
            CompoundCommandNode::Subshell(body) => {
                CompoundCommand::Subshell(self.materialize_stmt_seq(*body))
            }
            CompoundCommandNode::BraceGroup(body) => {
                CompoundCommand::BraceGroup(self.materialize_stmt_seq(*body))
            }
            CompoundCommandNode::Arithmetic(command) => {
                CompoundCommand::Arithmetic(crate::ArithmeticCommand {
                    span,
                    left_paren_span: command.left_paren_span,
                    expr_span: command.expr_span,
                    expr_ast: command
                        .expr_ast
                        .as_ref()
                        .map(|expr| self.materialize_arithmetic_expr(expr)),
                    right_paren_span: command.right_paren_span,
                })
            }
            CompoundCommandNode::Time {
                posix_format,
                command,
            } => CompoundCommand::Time(crate::TimeCommand {
                posix_format: *posix_format,
                command: command.map(|id| Box::new(self.materialize_single_stmt(id))),
                span,
            }),
            CompoundCommandNode::Conditional(command) => {
                CompoundCommand::Conditional(crate::ConditionalCommand {
                    expression: self.materialize_conditional_expr(&command.expression),
                    span,
                    left_bracket_span: command.left_bracket_span,
                    right_bracket_span: command.right_bracket_span,
                })
            }
            CompoundCommandNode::Coproc {
                name,
                name_span,
                body,
            } => CompoundCommand::Coproc(crate::CoprocCommand {
                name: name.clone(),
                name_span: *name_span,
                body: Box::new(self.materialize_single_stmt(*body)),
                span,
            }),
            CompoundCommandNode::Always { body, always_body } => {
                CompoundCommand::Always(crate::AlwaysCommand {
                    body: self.materialize_stmt_seq(*body),
                    always_body: self.materialize_stmt_seq(*always_body),
                    span,
                })
            }
        }
    }

    fn materialize_single_stmt(&self, id: StmtSeqId) -> Stmt {
        let mut stmts = self.materialize_stmt_seq(id).stmts.into_iter();
        let Some(stmt) = stmts.next() else {
            panic!("statement wrapper sequence contains one statement");
        };
        stmt
    }

    fn materialize_word(&self, id: WordId) -> Word {
        let node = &self.words[id.index()];
        Word {
            parts: self
                .word_part_lists
                .get(node.parts)
                .iter()
                .map(|part| self.materialize_word_part(part))
                .collect(),
            span: node.span,
            brace_syntax: self.brace_syntax_lists.get(node.brace_syntax).to_vec(),
        }
    }

    fn materialize_word_part(&self, node: &WordPartArenaNode) -> WordPartNode {
        WordPartNode {
            kind: match &node.kind {
                WordPartArena::Literal(text) => WordPart::Literal(text.clone()),
                WordPartArena::ZshQualifiedGlob(glob) => {
                    WordPart::ZshQualifiedGlob(self.materialize_zsh_qualified_glob(glob))
                }
                WordPartArena::SingleQuoted { value, dollar } => WordPart::SingleQuoted {
                    value: value.clone(),
                    dollar: *dollar,
                },
                WordPartArena::DoubleQuoted { parts, dollar } => WordPart::DoubleQuoted {
                    parts: self
                        .word_part_lists
                        .get(*parts)
                        .iter()
                        .map(|part| self.materialize_word_part(part))
                        .collect(),
                    dollar: *dollar,
                },
                WordPartArena::Variable(name) => WordPart::Variable(name.clone()),
                WordPartArena::CommandSubstitution { body, syntax } => {
                    WordPart::CommandSubstitution {
                        body: self.materialize_stmt_seq(*body),
                        syntax: *syntax,
                    }
                }
                WordPartArena::ArithmeticExpansion {
                    expression,
                    expression_ast,
                    expression_word_ast,
                    syntax,
                } => WordPart::ArithmeticExpansion {
                    expression: expression.clone(),
                    expression_ast: expression_ast
                        .as_ref()
                        .map(|expr| self.materialize_arithmetic_expr(expr)),
                    expression_word_ast: self.materialize_word(*expression_word_ast),
                    syntax: *syntax,
                },
                WordPartArena::Parameter(expansion) => {
                    WordPart::Parameter(self.materialize_parameter_expansion(expansion))
                }
                WordPartArena::ParameterExpansion {
                    reference,
                    operator,
                    operand,
                    operand_word_ast,
                    colon_variant,
                } => WordPart::ParameterExpansion {
                    reference: self.materialize_var_ref(reference),
                    operator: operator.clone(),
                    operand: operand.clone(),
                    operand_word_ast: operand_word_ast.map(|word| self.materialize_word(word)),
                    colon_variant: *colon_variant,
                },
                WordPartArena::Length(reference) => {
                    WordPart::Length(self.materialize_var_ref(reference))
                }
                WordPartArena::ArrayAccess(reference) => {
                    WordPart::ArrayAccess(self.materialize_var_ref(reference))
                }
                WordPartArena::ArrayLength(reference) => {
                    WordPart::ArrayLength(self.materialize_var_ref(reference))
                }
                WordPartArena::ArrayIndices(reference) => {
                    WordPart::ArrayIndices(self.materialize_var_ref(reference))
                }
                WordPartArena::Substring {
                    reference,
                    offset,
                    offset_ast,
                    offset_word_ast,
                    length,
                    length_ast,
                    length_word_ast,
                } => WordPart::Substring {
                    reference: self.materialize_var_ref(reference),
                    offset: offset.clone(),
                    offset_ast: offset_ast
                        .as_ref()
                        .map(|expr| self.materialize_arithmetic_expr(expr)),
                    offset_word_ast: self.materialize_word(*offset_word_ast),
                    length: length.clone(),
                    length_ast: length_ast
                        .as_ref()
                        .map(|expr| self.materialize_arithmetic_expr(expr)),
                    length_word_ast: length_word_ast.map(|word| self.materialize_word(word)),
                },
                WordPartArena::ArraySlice {
                    reference,
                    offset,
                    offset_ast,
                    offset_word_ast,
                    length,
                    length_ast,
                    length_word_ast,
                } => WordPart::ArraySlice {
                    reference: self.materialize_var_ref(reference),
                    offset: offset.clone(),
                    offset_ast: offset_ast
                        .as_ref()
                        .map(|expr| self.materialize_arithmetic_expr(expr)),
                    offset_word_ast: self.materialize_word(*offset_word_ast),
                    length: length.clone(),
                    length_ast: length_ast
                        .as_ref()
                        .map(|expr| self.materialize_arithmetic_expr(expr)),
                    length_word_ast: length_word_ast.map(|word| self.materialize_word(word)),
                },
                WordPartArena::IndirectExpansion {
                    reference,
                    operator,
                    operand,
                    operand_word_ast,
                    colon_variant,
                } => WordPart::IndirectExpansion {
                    reference: self.materialize_var_ref(reference),
                    operator: operator.clone(),
                    operand: operand.clone(),
                    operand_word_ast: operand_word_ast.map(|word| self.materialize_word(word)),
                    colon_variant: *colon_variant,
                },
                WordPartArena::PrefixMatch { prefix, kind } => WordPart::PrefixMatch {
                    prefix: prefix.clone(),
                    kind: *kind,
                },
                WordPartArena::ProcessSubstitution { body, is_input } => {
                    WordPart::ProcessSubstitution {
                        body: self.materialize_stmt_seq(*body),
                        is_input: *is_input,
                    }
                }
                WordPartArena::Transformation {
                    reference,
                    operator,
                } => WordPart::Transformation {
                    reference: self.materialize_var_ref(reference),
                    operator: *operator,
                },
            },
            span: node.span,
        }
    }
}

/// File-level arena node.
#[derive(Debug, Clone)]
pub struct FileNode {
    /// Root statement sequence.
    pub body: StmtSeqId,
    /// Source span of the file.
    pub span: Span,
}

/// Statement-sequence arena node.
#[derive(Debug, Clone)]
pub struct StmtSeqNode {
    /// Comments before the first statement in this sequence.
    pub leading_comments: IdRange<Comment>,
    /// Statements in source order.
    pub stmts: IdRange<StmtId>,
    /// Comments after the final statement and before the enclosing terminator.
    pub trailing_comments: IdRange<Comment>,
    /// Source span covering the full sequence.
    pub span: Span,
}

/// Statement arena node.
#[derive(Debug, Clone)]
pub struct StmtNode {
    /// Own-line comments immediately preceding this statement.
    pub leading_comments: IdRange<Comment>,
    /// Statement command payload.
    pub command: CommandId,
    /// Whether this statement was prefixed with `!`.
    pub negated: bool,
    /// Statement redirections.
    pub redirects: IdRange<RedirectNode>,
    /// Words found under statement-level redirections.
    pub redirect_words: IdRange<WordId>,
    /// Nested statement sequences found under statement-level redirections.
    pub redirect_child_sequences: IdRange<StmtSeqId>,
    /// Optional statement terminator.
    pub terminator: Option<StmtTerminator>,
    /// Source span of the terminator token when present.
    pub terminator_span: Option<Span>,
    /// Trailing inline comment on the statement line.
    pub inline_comment: Option<Comment>,
    /// Source span of the full statement.
    pub span: Span,
}

/// Arena-native statement redirection.
#[derive(Debug, Clone)]
pub struct RedirectNode {
    /// File descriptor (defaulted by redirect kind downstream).
    pub fd: Option<i32>,
    /// Variable name for `{var}` fd-variable redirects.
    pub fd_var: Option<crate::Name>,
    /// Source span of `{name}` in fd-variable redirects.
    pub fd_var_span: Option<Span>,
    /// Type of redirection.
    pub kind: crate::RedirectKind,
    /// Source span of this redirection.
    pub span: Span,
    /// Redirect operand payload.
    pub target: RedirectTargetNode,
}

/// Arena-native redirect operand.
#[derive(Debug, Clone)]
pub enum RedirectTargetNode {
    /// Standard redirect operand.
    Word(WordId),
    /// Heredoc delimiter metadata plus decoded body.
    Heredoc(HeredocNode),
}

/// Arena-native heredoc metadata and decoded body.
#[derive(Debug, Clone)]
pub struct HeredocNode {
    /// Parsed heredoc delimiter.
    pub delimiter: HeredocDelimiterNode,
    /// Parsed heredoc body.
    pub body: HeredocBodyNode,
}

/// Arena-native heredoc delimiter metadata.
#[derive(Debug, Clone)]
pub struct HeredocDelimiterNode {
    /// Raw delimiter word with original quoting preserved.
    pub raw: WordId,
    /// Cooked delimiter string after quote removal.
    pub cooked: String,
    /// Source span of the delimiter token.
    pub span: Span,
    /// Whether the delimiter used shell quoting.
    pub quoted: bool,
    /// Whether the body should be decoded for expansions.
    pub expands_body: bool,
    /// Whether `<<-` tab stripping applies.
    pub strip_tabs: bool,
}

/// Arena-native heredoc body.
#[derive(Debug, Clone)]
pub struct HeredocBodyNode {
    /// Expansion mode for the body.
    pub mode: crate::HeredocBodyMode,
    /// Whether body text points back into source.
    pub source_backed: bool,
    /// Body parts in source order.
    pub parts: IdRange<ArenaHeredocBodyPartNode>,
    /// Source span of the body.
    pub span: Span,
}

/// Arena-native heredoc body part paired with its source span.
#[derive(Debug, Clone)]
pub struct ArenaHeredocBodyPartNode {
    /// Body part payload.
    pub kind: ArenaHeredocBodyPart,
    /// Source span of the body part.
    pub span: Span,
}

/// Arena-native heredoc body part payload.
#[derive(Debug, Clone)]
pub enum ArenaHeredocBodyPart {
    /// Literal heredoc text.
    Literal(crate::LiteralText),
    /// Simple variable expansion.
    Variable(crate::Name),
    /// Command substitution body.
    CommandSubstitution {
        body: StmtSeqId,
        syntax: crate::CommandSubstitutionSyntax,
    },
    /// Arithmetic expansion body.
    ArithmeticExpansion {
        expression: crate::SourceText,
        expression_ast: Option<ArithmeticExprArenaNode>,
        expression_word_ast: WordId,
        syntax: crate::ArithmeticExpansionSyntax,
    },
    /// Parameter expansion payload.
    Parameter(Box<ParameterExpansionNode>),
}

/// Arena-native variable assignment.
#[derive(Debug, Clone)]
pub struct AssignmentNode {
    /// Assignment target.
    pub target: VarRefNode,
    /// Assignment value.
    pub value: AssignmentValueNode,
    /// Whether this is an append assignment.
    pub append: bool,
    /// Source span of this assignment.
    pub span: Span,
}

/// Arena-native declaration operand.
#[derive(Debug, Clone)]
pub enum DeclOperandNode {
    /// A literal option word such as `-a` or `+x`.
    Flag(WordId),
    /// A bare variable name or indexed reference.
    Name(VarRefNode),
    /// A typed assignment operand.
    Assignment(AssignmentNode),
    /// A word whose runtime expansion may produce a flag, name, or assignment.
    Dynamic(WordId),
}

/// Arena-native assignment value.
#[derive(Debug, Clone)]
pub enum AssignmentValueNode {
    /// Scalar assignment value.
    Scalar(WordId),
    /// Compound array assignment value.
    Compound(ArrayExprNode),
}

/// Arena-native variable reference.
#[derive(Debug, Clone)]
pub struct VarRefNode {
    /// Variable name.
    pub name: crate::Name,
    /// Source span of the variable name.
    pub name_span: Span,
    /// Optional array subscript.
    pub subscript: Option<Box<SubscriptNode>>,
    /// Source span of the full reference.
    pub span: Span,
}

/// Arena-native array subscript.
#[derive(Debug, Clone)]
pub struct SubscriptNode {
    /// Cooked subscript text.
    pub text: crate::SourceText,
    /// Original subscript syntax when it differs from the cooked semantic text.
    pub raw: Option<crate::SourceText>,
    /// Syntactic subscript shape.
    pub kind: crate::SubscriptKind,
    /// Downstream interpretation for the subscript.
    pub interpretation: crate::SubscriptInterpretation,
    /// Parsed word view of the original subscript syntax.
    pub word_ast: Option<WordId>,
    /// Typed arithmetic view of this subscript when available.
    pub arithmetic_ast: Option<ArithmeticExprArenaNode>,
}

/// Arena-native compound array literal.
#[derive(Debug, Clone)]
pub struct ArrayExprNode {
    /// Array flavor implied by parse context.
    pub kind: crate::ArrayKind,
    /// Array elements.
    pub elements: IdRange<ArrayElemNode>,
    /// Source span of the array expression.
    pub span: Span,
}

/// Arena-native array value word plus parser-owned surface metadata.
#[derive(Debug, Clone)]
pub struct ArrayValueWordNode {
    /// Array value word.
    pub word: WordId,
    /// Whether the value has a top-level unquoted comma.
    pub has_top_level_unquoted_comma: bool,
}

/// Arena-native compound array element.
#[derive(Debug, Clone)]
pub enum ArrayElemNode {
    /// Positional array element.
    Sequential(ArrayValueWordNode),
    /// Keyed array element.
    Keyed {
        key: SubscriptNode,
        value: ArrayValueWordNode,
    },
    /// Append to an existing key.
    KeyedAppend {
        key: SubscriptNode,
        value: ArrayValueWordNode,
    },
}

/// Command arena node.
#[derive(Debug, Clone)]
pub struct CommandNode {
    /// Coarse command kind for cheap filtering.
    pub kind: ArenaFileCommandKind,
    /// Source span of the command.
    pub span: Span,
    /// Words found under this command.
    pub words: IdRange<WordId>,
    /// Nested statement sequences found under this command.
    pub child_sequences: IdRange<StmtSeqId>,
    /// Native arena payload for command families that have been migrated.
    pub payload: CommandNodePayload,
}

/// Arena-native command payloads.
#[derive(Debug, Clone)]
pub enum CommandNodePayload {
    /// Simple command payload.
    Simple(SimpleCommandNode),
    /// Typed builtin command payload.
    Builtin(BuiltinCommandNode),
    /// Declaration builtin command payload.
    Decl(DeclCommandNode),
    /// Binary shell command payload.
    Binary(BinaryCommandNode),
    /// Function definition command payload.
    Function(FunctionCommandNode),
    /// Anonymous zsh function command payload.
    AnonymousFunction(AnonymousFunctionCommandNode),
    /// Compound shell command payload.
    Compound(CompoundCommandNode),
}

/// Arena-native simple command payload.
#[derive(Debug, Clone)]
pub struct SimpleCommandNode {
    /// Command name word.
    pub name: WordId,
    /// Command argument words.
    pub args: IdRange<WordId>,
    /// Prefix assignments.
    pub assignments: IdRange<AssignmentNode>,
}

/// Arena-native typed builtin command payload.
#[derive(Debug, Clone)]
pub struct BuiltinCommandNode {
    /// Builtin command kind.
    pub kind: BuiltinCommandNodeKind,
    /// Optional primary operand (`depth` or `code` depending on the builtin).
    pub primary: Option<WordId>,
    /// Additional operands preserved for fidelity.
    pub extra_args: IdRange<WordId>,
    /// Prefix assignments.
    pub assignments: IdRange<AssignmentNode>,
}

/// Arena-native typed builtin command kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinCommandNodeKind {
    /// `break [N]`.
    Break,
    /// `continue [N]`.
    Continue,
    /// `return [N]`.
    Return,
    /// `exit [N]`.
    Exit,
}

/// Arena-native declaration command payload.
#[derive(Debug, Clone)]
pub struct DeclCommandNode {
    /// Declaration builtin variant.
    pub variant: crate::Name,
    /// Source span of the declaration builtin name.
    pub variant_span: Span,
    /// Parsed declaration operands.
    pub operands: IdRange<DeclOperandNode>,
    /// Prefix assignments.
    pub assignments: IdRange<AssignmentNode>,
}

/// Arena-native binary command payload.
#[derive(Debug, Clone)]
pub struct BinaryCommandNode {
    /// Left-hand statement sequence.
    pub left: StmtSeqId,
    /// Binary operator.
    pub op: crate::BinaryOp,
    /// Source span of the operator token.
    pub op_span: Span,
    /// Right-hand statement sequence.
    pub right: StmtSeqId,
}

/// Arena-native named function command payload.
#[derive(Debug, Clone)]
pub struct FunctionCommandNode {
    /// Source span of the `function` keyword when present.
    pub function_keyword_span: Option<Span>,
    /// Parsed function header entries.
    pub entries: IdRange<FunctionHeaderEntryNode>,
    /// Source span of trailing `()` when present.
    pub trailing_parens_span: Option<Span>,
    /// Function body sequence.
    pub body: StmtSeqId,
}

/// Arena-native function header entry.
#[derive(Debug, Clone)]
pub struct FunctionHeaderEntryNode {
    /// Header entry word.
    pub word: WordId,
    /// Static function name when one could be recovered.
    pub static_name: Option<crate::Name>,
}

/// Arena-native anonymous function command payload.
#[derive(Debug, Clone)]
pub struct AnonymousFunctionCommandNode {
    /// Preserved anonymous function surface.
    pub surface: crate::AnonymousFunctionSurface,
    /// Anonymous function body sequence.
    pub body: StmtSeqId,
    /// Invocation argument words.
    pub args: IdRange<WordId>,
}

/// Arena-native compound command payload.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum CompoundCommandNode {
    /// If statement.
    If {
        condition: StmtSeqId,
        then_branch: StmtSeqId,
        elif_branches: IdRange<ElifBranchNode>,
        else_branch: Option<StmtSeqId>,
        syntax: crate::IfSyntax,
    },
    /// For loop.
    For {
        targets: IdRange<ForTargetNode>,
        words: Option<IdRange<WordId>>,
        body: StmtSeqId,
        syntax: crate::ForSyntax,
    },
    /// Zsh repeat loop.
    Repeat {
        count: WordId,
        body: StmtSeqId,
        syntax: crate::RepeatSyntax,
    },
    /// Zsh foreach loop.
    Foreach {
        variable: crate::Name,
        variable_span: Span,
        words: IdRange<WordId>,
        body: StmtSeqId,
        syntax: crate::ForeachSyntax,
    },
    /// C-style arithmetic for loop.
    ArithmeticFor(Box<ArithmeticForCommandNode>),
    /// While loop.
    While {
        condition: StmtSeqId,
        body: StmtSeqId,
    },
    /// Until loop.
    Until {
        condition: StmtSeqId,
        body: StmtSeqId,
    },
    /// Case statement.
    Case {
        word: WordId,
        cases: IdRange<CaseItemNode>,
    },
    /// Select loop.
    Select {
        variable: crate::Name,
        variable_span: Span,
        words: IdRange<WordId>,
        body: StmtSeqId,
    },
    /// Subshell.
    Subshell(StmtSeqId),
    /// Brace group.
    BraceGroup(StmtSeqId),
    /// Arithmetic command.
    Arithmetic(ArithmeticCommandNode),
    /// Time command.
    Time {
        posix_format: bool,
        command: Option<StmtSeqId>,
    },
    /// Conditional expression command.
    Conditional(ConditionalCommandNode),
    /// Coprocess command.
    Coproc {
        name: crate::Name,
        name_span: Option<Span>,
        body: StmtSeqId,
    },
    /// Zsh always/finally-style cleanup block.
    Always {
        body: StmtSeqId,
        always_body: StmtSeqId,
    },
}

/// Arena-native if/elif branch pair.
#[derive(Debug, Clone)]
pub struct ElifBranchNode {
    /// Elif condition sequence.
    pub condition: StmtSeqId,
    /// Elif body sequence.
    pub body: StmtSeqId,
}

/// Arena-native for-loop target.
#[derive(Debug, Clone)]
pub struct ForTargetNode {
    /// Source-preserving target word.
    pub word: WordId,
    /// Normalized identifier when the target is a plain shell name.
    pub name: Option<crate::Name>,
    /// Source span of the target.
    pub span: Span,
}

/// Arena-native case item.
#[derive(Debug, Clone)]
pub struct CaseItemNode {
    /// Case patterns.
    pub patterns: IdRange<PatternNode>,
    /// Case body sequence.
    pub body: StmtSeqId,
    /// Case terminator.
    pub terminator: crate::CaseTerminator,
    /// Source span of the case terminator token when present.
    pub terminator_span: Option<Span>,
}

/// Arena-native shell pattern.
#[derive(Debug, Clone)]
pub struct PatternNode {
    /// Pattern parts in source order.
    pub parts: IdRange<PatternPartArenaNode>,
    /// Source span of the full pattern.
    pub span: Span,
}

/// Arena-native pattern part paired with its source span.
#[derive(Debug, Clone)]
pub struct PatternPartArenaNode {
    /// Pattern part payload.
    pub kind: PatternPartArena,
    /// Source span of this pattern part.
    pub span: Span,
}

/// Arena-native pattern part payload.
#[derive(Debug, Clone)]
pub enum PatternPartArena {
    /// Literal pattern text.
    Literal(crate::LiteralText),
    /// `*`.
    AnyString,
    /// `?`.
    AnyChar,
    /// Bracket character class source text.
    CharClass(crate::SourceText),
    /// Extended glob group.
    Group {
        kind: crate::PatternGroupKind,
        patterns: IdRange<PatternNode>,
    },
    /// Word-backed pattern fragment.
    Word(WordId),
}

/// Arena-native arithmetic command.
#[derive(Debug, Clone)]
pub struct ArithmeticCommandNode {
    pub left_paren_span: Span,
    pub expr_span: Option<Span>,
    pub expr_ast: Option<ArithmeticExprArenaNode>,
    pub right_paren_span: Span,
}

/// Arena-native arithmetic-for command.
#[derive(Debug, Clone)]
pub struct ArithmeticForCommandNode {
    pub left_paren_span: Span,
    pub init_span: Option<Span>,
    pub init_ast: Option<ArithmeticExprArenaNode>,
    pub first_semicolon_span: Span,
    pub condition_span: Option<Span>,
    pub condition_ast: Option<ArithmeticExprArenaNode>,
    pub second_semicolon_span: Span,
    pub step_span: Option<Span>,
    pub step_ast: Option<ArithmeticExprArenaNode>,
    pub right_paren_span: Span,
    pub body: StmtSeqId,
}

/// Arena-native conditional command.
#[derive(Debug, Clone)]
pub struct ConditionalCommandNode {
    pub expression: ConditionalExprArena,
    pub left_bracket_span: Span,
    pub right_bracket_span: Span,
}

/// Arena-native conditional expression.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum ConditionalExprArena {
    Binary {
        left: Box<ConditionalExprArena>,
        op: crate::ConditionalBinaryOp,
        op_span: Span,
        right: Box<ConditionalExprArena>,
    },
    Unary {
        op: crate::ConditionalUnaryOp,
        op_span: Span,
        expr: Box<ConditionalExprArena>,
    },
    Parenthesized {
        left_paren_span: Span,
        expr: Box<ConditionalExprArena>,
        right_paren_span: Span,
    },
    Word(WordId),
    Pattern(PatternNode),
    Regex(WordId),
    VarRef(VarRefNode),
}

/// Arena-native arithmetic expression plus its source span.
#[derive(Debug, Clone)]
pub struct ArithmeticExprArenaNode {
    pub kind: ArithmeticExprArena,
    pub span: Span,
}

/// Arena-native arithmetic expression.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum ArithmeticExprArena {
    Number(crate::SourceText),
    Variable(crate::Name),
    Indexed {
        name: crate::Name,
        index: Box<ArithmeticExprArenaNode>,
    },
    ShellWord(WordId),
    Parenthesized {
        expression: Box<ArithmeticExprArenaNode>,
    },
    Unary {
        op: crate::ArithmeticUnaryOp,
        expr: Box<ArithmeticExprArenaNode>,
    },
    Postfix {
        expr: Box<ArithmeticExprArenaNode>,
        op: crate::ArithmeticPostfixOp,
    },
    Binary {
        left: Box<ArithmeticExprArenaNode>,
        op: crate::ArithmeticBinaryOp,
        right: Box<ArithmeticExprArenaNode>,
    },
    Conditional {
        condition: Box<ArithmeticExprArenaNode>,
        then_expr: Box<ArithmeticExprArenaNode>,
        else_expr: Box<ArithmeticExprArenaNode>,
    },
    Assignment {
        target: ArithmeticLvalueArena,
        op: crate::ArithmeticAssignOp,
        value: Box<ArithmeticExprArenaNode>,
    },
}

/// Arena-native arithmetic assignment target.
#[derive(Debug, Clone)]
pub enum ArithmeticLvalueArena {
    Variable(crate::Name),
    Indexed {
        name: crate::Name,
        index: Box<ArithmeticExprArenaNode>,
    },
}

/// Coarse command family stored with command arena nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArenaFileCommandKind {
    /// Simple command.
    Simple,
    /// Builtin command with a dedicated node.
    Builtin,
    /// Declaration builtin clause.
    Decl,
    /// Binary shell operator.
    Binary,
    /// Compound command.
    Compound,
    /// Function definition.
    Function,
    /// Anonymous zsh function.
    AnonymousFunction,
}

/// Word arena node.
#[derive(Debug, Clone)]
pub struct WordNode {
    /// Word parts in source order.
    pub parts: IdRange<WordPartArenaNode>,
    /// Source span of this word.
    pub span: Span,
    /// Brace-syntax facts attached by the parser.
    pub brace_syntax: IdRange<crate::BraceSyntax>,
}

/// Arena-native word part paired with its source span.
#[derive(Debug, Clone)]
pub struct WordPartArenaNode {
    /// Word part payload.
    pub kind: WordPartArena,
    /// Source span of this word part.
    pub span: Span,
}

/// Arena-native word part payload.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum WordPartArena {
    /// Literal text.
    Literal(crate::LiteralText),
    /// Zsh glob with one classic trailing qualifier group.
    ZshQualifiedGlob(ZshQualifiedGlobNode),
    /// Single-quoted literal content.
    SingleQuoted {
        value: crate::SourceText,
        dollar: bool,
    },
    /// Double-quoted content with nested parts.
    DoubleQuoted {
        parts: IdRange<WordPartArenaNode>,
        dollar: bool,
    },
    /// Variable expansion.
    Variable(crate::Name),
    /// Command substitution.
    CommandSubstitution {
        body: StmtSeqId,
        syntax: crate::CommandSubstitutionSyntax,
    },
    /// Arithmetic expansion.
    ArithmeticExpansion {
        expression: crate::SourceText,
        expression_ast: Option<ArithmeticExprArenaNode>,
        expression_word_ast: WordId,
        syntax: crate::ArithmeticExpansionSyntax,
    },
    /// Unified parameter-expansion family for `${...}` forms.
    Parameter(ParameterExpansionNode),
    /// Parameter expansion with operator.
    ParameterExpansion {
        reference: VarRefNode,
        operator: crate::ParameterOp,
        operand: Option<crate::SourceText>,
        operand_word_ast: Option<WordId>,
        colon_variant: bool,
    },
    /// Length expansion.
    Length(VarRefNode),
    /// Array element access.
    ArrayAccess(VarRefNode),
    /// Array length.
    ArrayLength(VarRefNode),
    /// Array indices.
    ArrayIndices(VarRefNode),
    /// Substring extraction.
    Substring {
        reference: VarRefNode,
        offset: crate::SourceText,
        offset_ast: Option<ArithmeticExprArenaNode>,
        offset_word_ast: WordId,
        length: Option<crate::SourceText>,
        length_ast: Option<ArithmeticExprArenaNode>,
        length_word_ast: Option<WordId>,
    },
    /// Array slice.
    ArraySlice {
        reference: VarRefNode,
        offset: crate::SourceText,
        offset_ast: Option<ArithmeticExprArenaNode>,
        offset_word_ast: WordId,
        length: Option<crate::SourceText>,
        length_ast: Option<ArithmeticExprArenaNode>,
        length_word_ast: Option<WordId>,
    },
    /// Indirect expansion.
    IndirectExpansion {
        reference: VarRefNode,
        operator: Option<crate::ParameterOp>,
        operand: Option<crate::SourceText>,
        operand_word_ast: Option<WordId>,
        colon_variant: bool,
    },
    /// Prefix matching.
    PrefixMatch {
        prefix: crate::Name,
        kind: crate::PrefixMatchKind,
    },
    /// Process substitution.
    ProcessSubstitution { body: StmtSeqId, is_input: bool },
    /// Parameter transformation.
    Transformation {
        reference: VarRefNode,
        operator: char,
    },
}

/// Arena-native zsh qualified glob.
#[derive(Debug, Clone)]
pub struct ZshQualifiedGlobNode {
    pub span: Span,
    pub segments: IdRange<ZshGlobSegmentNode>,
    pub qualifiers: Option<ZshGlobQualifierGroupNode>,
}

/// Arena-native zsh glob segment.
#[derive(Debug, Clone)]
pub enum ZshGlobSegmentNode {
    Pattern(PatternNode),
    InlineControl(crate::ZshInlineGlobControl),
}

/// Arena-native zsh glob qualifier group.
#[derive(Debug, Clone)]
pub struct ZshGlobQualifierGroupNode {
    pub span: Span,
    pub kind: crate::ZshGlobQualifierKind,
    pub fragments: IdRange<crate::ZshGlobQualifier>,
}

/// Arena-native parameter expansion.
#[derive(Debug, Clone)]
pub struct ParameterExpansionNode {
    /// Expansion syntax family.
    pub syntax: ParameterExpansionSyntaxNode,
    /// Source span of the expansion.
    pub span: Span,
    /// Raw body text.
    pub raw_body: crate::SourceText,
}

/// Arena-native parameter expansion syntax.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum ParameterExpansionSyntaxNode {
    Bourne(BourneParameterExpansionNode),
    Zsh(ZshParameterExpansionNode),
}

/// Arena-native Bourne-style parameter expansion.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum BourneParameterExpansionNode {
    Access {
        reference: VarRefNode,
    },
    Length {
        reference: VarRefNode,
    },
    Indices {
        reference: VarRefNode,
    },
    Indirect {
        reference: VarRefNode,
        operator: Option<crate::ParameterOp>,
        operand: Option<crate::SourceText>,
        operand_word_ast: Option<WordId>,
        colon_variant: bool,
    },
    PrefixMatch {
        prefix: crate::Name,
        kind: crate::PrefixMatchKind,
    },
    Slice {
        reference: VarRefNode,
        offset: crate::SourceText,
        offset_ast: Option<ArithmeticExprArenaNode>,
        offset_word_ast: WordId,
        length: Option<crate::SourceText>,
        length_ast: Option<ArithmeticExprArenaNode>,
        length_word_ast: Option<WordId>,
    },
    Operation {
        reference: VarRefNode,
        operator: crate::ParameterOp,
        operand: Option<crate::SourceText>,
        operand_word_ast: Option<WordId>,
        colon_variant: bool,
    },
    Transformation {
        reference: VarRefNode,
        operator: char,
    },
}

/// Arena-native zsh parameter expansion.
#[derive(Debug, Clone)]
pub struct ZshParameterExpansionNode {
    pub target: ZshExpansionTargetNode,
    pub modifiers: IdRange<ZshModifierNode>,
    pub length_prefix: Option<Span>,
    pub operation: Option<ZshExpansionOperationNode>,
}

/// Arena-native zsh expansion target.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum ZshExpansionTargetNode {
    Reference(VarRefNode),
    Nested(Box<ParameterExpansionNode>),
    Word(WordId),
    Empty,
}

/// Arena-native zsh modifier.
#[derive(Debug, Clone)]
pub struct ZshModifierNode {
    pub name: char,
    pub argument: Option<crate::SourceText>,
    pub argument_word_ast: Option<WordId>,
    pub argument_delimiter: Option<char>,
    pub span: Span,
}

/// Arena-native zsh parameter operation.
#[derive(Debug, Clone)]
pub enum ZshExpansionOperationNode {
    PatternOperation {
        kind: crate::ZshPatternOp,
        operand: crate::SourceText,
        operand_word_ast: WordId,
    },
    Defaulting {
        kind: crate::ZshDefaultingOp,
        operand: crate::SourceText,
        operand_word_ast: WordId,
        colon_variant: bool,
    },
    TrimOperation {
        kind: crate::ZshTrimOp,
        operand: crate::SourceText,
        operand_word_ast: WordId,
    },
    ReplacementOperation {
        kind: crate::ZshReplacementOp,
        pattern: crate::SourceText,
        pattern_word_ast: WordId,
        replacement: Option<crate::SourceText>,
        replacement_word_ast: Option<WordId>,
    },
    Slice {
        offset: crate::SourceText,
        offset_word_ast: WordId,
        length: Option<crate::SourceText>,
        length_word_ast: Option<WordId>,
    },
    Unknown {
        text: crate::SourceText,
        word_ast: WordId,
    },
}

/// Borrowed view of a file node.
#[derive(Debug, Clone, Copy)]
pub struct FileView<'a> {
    store: &'a AstStore,
    id: FileId,
}

impl<'a> FileView<'a> {
    /// Returns this node's ID.
    pub fn id(self) -> FileId {
        self.id
    }

    /// Returns the root statement sequence.
    pub fn body(self) -> StmtSeqView<'a> {
        self.store.stmt_seq(self.store.files[self.id.index()].body)
    }

    /// Returns the file source span.
    pub fn span(self) -> Span {
        self.store.files[self.id.index()].span
    }
}

/// Borrowed view of a statement sequence node.
#[derive(Debug, Clone, Copy)]
pub struct StmtSeqView<'a> {
    store: &'a AstStore,
    id: StmtSeqId,
}

impl<'a> StmtSeqView<'a> {
    /// Returns this node's ID.
    pub fn id(self) -> StmtSeqId {
        self.id
    }

    /// Returns the raw statement IDs in this sequence.
    pub fn stmt_ids(self) -> &'a [StmtId] {
        self.store.stmt_id_lists.get(self.node().stmts)
    }

    /// Returns the statements in this sequence.
    pub fn stmts(self) -> impl ExactSizeIterator<Item = StmtView<'a>> + 'a {
        self.stmt_ids()
            .iter()
            .copied()
            .map(move |id| self.store.stmt(id))
    }

    /// Returns leading comments for this sequence.
    pub fn leading_comments(self) -> &'a [Comment] {
        self.store.comment_lists.get(self.node().leading_comments)
    }

    /// Returns trailing comments for this sequence.
    pub fn trailing_comments(self) -> &'a [Comment] {
        self.store.comment_lists.get(self.node().trailing_comments)
    }

    /// Returns this sequence's source span.
    pub fn span(self) -> Span {
        self.node().span
    }

    fn node(self) -> &'a StmtSeqNode {
        &self.store.stmt_seqs[self.id.index()]
    }
}

/// Borrowed view of a statement node.
#[derive(Debug, Clone, Copy)]
pub struct StmtView<'a> {
    store: &'a AstStore,
    id: StmtId,
}

impl<'a> StmtView<'a> {
    /// Returns this node's ID.
    pub fn id(self) -> StmtId {
        self.id
    }

    /// Returns this statement's command.
    pub fn command(self) -> CommandView<'a> {
        self.store.command(self.node().command)
    }

    /// Returns comments immediately preceding this statement.
    pub fn leading_comments(self) -> &'a [Comment] {
        self.store.comment_lists.get(self.node().leading_comments)
    }

    /// Returns this statement's redirections.
    pub fn redirects(self) -> &'a [RedirectNode] {
        self.store.redirect_lists.get(self.node().redirects)
    }

    /// Returns IDs for words found under statement redirections.
    pub fn redirect_word_ids(self) -> &'a [WordId] {
        self.store.word_id_lists.get(self.node().redirect_words)
    }

    /// Returns IDs for nested statement sequences found under statement redirections.
    pub fn redirect_child_sequence_ids(self) -> &'a [StmtSeqId] {
        self.store
            .stmt_seq_id_lists
            .get(self.node().redirect_child_sequences)
    }

    /// Returns nested statement sequences found under statement redirections.
    pub fn redirect_child_sequences(self) -> impl ExactSizeIterator<Item = StmtSeqView<'a>> + 'a {
        self.redirect_child_sequence_ids()
            .iter()
            .copied()
            .map(move |id| self.store.stmt_seq(id))
    }

    /// Returns whether this statement is negated.
    pub fn negated(self) -> bool {
        self.node().negated
    }

    /// Returns this statement's terminator.
    pub fn terminator(self) -> Option<StmtTerminator> {
        self.node().terminator
    }

    /// Returns this statement's inline comment.
    pub fn inline_comment(self) -> Option<Comment> {
        self.node().inline_comment
    }

    /// Returns this statement's source span.
    pub fn span(self) -> Span {
        self.node().span
    }

    /// Materializes this statement into the recursive AST representation.
    pub fn to_stmt(self) -> Stmt {
        self.store.materialize_stmt(self.id)
    }

    fn node(self) -> &'a StmtNode {
        &self.store.stmts[self.id.index()]
    }
}

/// Borrowed view of a command node.
#[derive(Debug, Clone, Copy)]
pub struct CommandView<'a> {
    store: &'a AstStore,
    id: CommandId,
}

impl<'a> CommandView<'a> {
    /// Returns this node's ID.
    pub fn id(self) -> CommandId {
        self.id
    }

    /// Returns this command's coarse kind.
    pub fn kind(self) -> ArenaFileCommandKind {
        self.node().kind
    }

    /// Returns this command's source span.
    pub fn span(self) -> Span {
        self.node().span
    }

    /// Returns IDs for words found under this command.
    pub fn word_ids(self) -> &'a [WordId] {
        self.store.word_id_lists.get(self.node().words)
    }

    /// Returns words found under this command.
    pub fn words(self) -> impl ExactSizeIterator<Item = WordView<'a>> + 'a {
        self.word_ids()
            .iter()
            .copied()
            .map(move |id| self.store.word(id))
    }

    /// Returns the native simple command payload when this command is simple.
    pub fn simple(self) -> Option<SimpleCommandView<'a>> {
        match &self.node().payload {
            CommandNodePayload::Simple(_) => Some(SimpleCommandView {
                store: self.store,
                id: self.id,
            }),
            CommandNodePayload::Builtin(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Binary(_)
            | CommandNodePayload::Function(_)
            | CommandNodePayload::AnonymousFunction(_)
            | CommandNodePayload::Compound(_) => None,
        }
    }

    /// Returns the native typed builtin payload when this command is a typed builtin.
    pub fn builtin(self) -> Option<BuiltinCommandView<'a>> {
        match &self.node().payload {
            CommandNodePayload::Builtin(_) => Some(BuiltinCommandView {
                store: self.store,
                id: self.id,
            }),
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Binary(_)
            | CommandNodePayload::Function(_)
            | CommandNodePayload::AnonymousFunction(_)
            | CommandNodePayload::Compound(_) => None,
        }
    }

    /// Returns the native declaration payload when this command is a declaration builtin.
    pub fn decl(self) -> Option<DeclCommandView<'a>> {
        match &self.node().payload {
            CommandNodePayload::Decl(_) => Some(DeclCommandView {
                store: self.store,
                id: self.id,
            }),
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Builtin(_)
            | CommandNodePayload::Binary(_)
            | CommandNodePayload::Function(_)
            | CommandNodePayload::AnonymousFunction(_)
            | CommandNodePayload::Compound(_) => None,
        }
    }

    /// Returns the native binary payload when this command is a binary shell command.
    pub fn binary(self) -> Option<BinaryCommandView<'a>> {
        match &self.node().payload {
            CommandNodePayload::Binary(_) => Some(BinaryCommandView {
                store: self.store,
                id: self.id,
            }),
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Builtin(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Function(_)
            | CommandNodePayload::AnonymousFunction(_)
            | CommandNodePayload::Compound(_) => None,
        }
    }

    /// Returns the native function payload when this command is a named function definition.
    pub fn function(self) -> Option<FunctionCommandView<'a>> {
        match &self.node().payload {
            CommandNodePayload::Function(_) => Some(FunctionCommandView {
                store: self.store,
                id: self.id,
            }),
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Builtin(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Binary(_)
            | CommandNodePayload::AnonymousFunction(_)
            | CommandNodePayload::Compound(_) => None,
        }
    }

    /// Returns the native anonymous function payload when this command is anonymous.
    pub fn anonymous_function(self) -> Option<AnonymousFunctionCommandView<'a>> {
        match &self.node().payload {
            CommandNodePayload::AnonymousFunction(_) => Some(AnonymousFunctionCommandView {
                store: self.store,
                id: self.id,
            }),
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Builtin(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Binary(_)
            | CommandNodePayload::Function(_)
            | CommandNodePayload::Compound(_) => None,
        }
    }

    /// Returns the native compound payload when this command is compound.
    pub fn compound(self) -> Option<CompoundCommandView<'a>> {
        match &self.node().payload {
            CommandNodePayload::Compound(_) => Some(CompoundCommandView {
                store: self.store,
                id: self.id,
            }),
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Builtin(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Binary(_)
            | CommandNodePayload::Function(_)
            | CommandNodePayload::AnonymousFunction(_) => None,
        }
    }

    /// Returns IDs for nested statement sequences found under this command.
    pub fn child_sequence_ids(self) -> &'a [StmtSeqId] {
        self.store
            .stmt_seq_id_lists
            .get(self.node().child_sequences)
    }

    /// Returns nested statement sequences found under this command.
    pub fn child_sequences(self) -> impl ExactSizeIterator<Item = StmtSeqView<'a>> + 'a {
        self.child_sequence_ids()
            .iter()
            .copied()
            .map(move |id| self.store.stmt_seq(id))
    }

    fn node(self) -> &'a CommandNode {
        &self.store.commands[self.id.index()]
    }
}

/// Borrowed view of an arena-native simple command payload.
#[derive(Debug, Clone, Copy)]
pub struct SimpleCommandView<'a> {
    store: &'a AstStore,
    id: CommandId,
}

impl<'a> SimpleCommandView<'a> {
    /// Returns the command name word.
    pub fn name(self) -> WordView<'a> {
        self.store.word(self.node().name)
    }

    /// Returns the command name word ID.
    pub fn name_id(self) -> WordId {
        self.node().name
    }

    /// Returns command argument word IDs.
    pub fn arg_ids(self) -> &'a [WordId] {
        self.store.word_id_lists.get(self.node().args)
    }

    /// Returns command argument words.
    pub fn args(self) -> impl ExactSizeIterator<Item = WordView<'a>> + 'a {
        self.arg_ids()
            .iter()
            .copied()
            .map(move |id| self.store.word(id))
    }

    /// Returns prefix assignments.
    pub fn assignments(self) -> &'a [AssignmentNode] {
        self.store.assignment_lists.get(self.node().assignments)
    }

    fn node(self) -> &'a SimpleCommandNode {
        match &self.store.commands[self.id.index()].payload {
            CommandNodePayload::Simple(command) => command,
            CommandNodePayload::Builtin(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Binary(_)
            | CommandNodePayload::Function(_)
            | CommandNodePayload::AnonymousFunction(_)
            | CommandNodePayload::Compound(_) => {
                unreachable!("simple view requires simple payload")
            }
        }
    }
}

/// Borrowed view of an arena-native typed builtin payload.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinCommandView<'a> {
    store: &'a AstStore,
    id: CommandId,
}

impl<'a> BuiltinCommandView<'a> {
    /// Returns the builtin command kind.
    pub fn kind(self) -> BuiltinCommandNodeKind {
        self.node().kind
    }

    /// Returns the primary operand word, if present.
    pub fn primary(self) -> Option<WordView<'a>> {
        self.node().primary.map(|id| self.store.word(id))
    }

    /// Returns the primary operand word ID, if present.
    pub fn primary_id(self) -> Option<WordId> {
        self.node().primary
    }

    /// Returns additional operand word IDs.
    pub fn extra_arg_ids(self) -> &'a [WordId] {
        self.store.word_id_lists.get(self.node().extra_args)
    }

    /// Returns additional operand words.
    pub fn extra_args(self) -> impl ExactSizeIterator<Item = WordView<'a>> + 'a {
        self.extra_arg_ids()
            .iter()
            .copied()
            .map(move |id| self.store.word(id))
    }

    /// Returns prefix assignments.
    pub fn assignments(self) -> &'a [AssignmentNode] {
        self.store.assignment_lists.get(self.node().assignments)
    }

    fn node(self) -> &'a BuiltinCommandNode {
        match &self.store.commands[self.id.index()].payload {
            CommandNodePayload::Builtin(command) => command,
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Binary(_)
            | CommandNodePayload::Function(_)
            | CommandNodePayload::AnonymousFunction(_)
            | CommandNodePayload::Compound(_) => {
                unreachable!("builtin view requires builtin payload")
            }
        }
    }
}

/// Borrowed view of an arena-native declaration command payload.
#[derive(Debug, Clone, Copy)]
pub struct DeclCommandView<'a> {
    store: &'a AstStore,
    id: CommandId,
}

impl<'a> DeclCommandView<'a> {
    /// Returns the declaration builtin variant.
    pub fn variant(self) -> &'a crate::Name {
        &self.node().variant
    }

    /// Returns the source span of the declaration builtin name.
    pub fn variant_span(self) -> Span {
        self.node().variant_span
    }

    /// Returns parsed declaration operands.
    pub fn operands(self) -> &'a [DeclOperandNode] {
        self.store.decl_operand_lists.get(self.node().operands)
    }

    /// Returns prefix assignments.
    pub fn assignments(self) -> &'a [AssignmentNode] {
        self.store.assignment_lists.get(self.node().assignments)
    }

    fn node(self) -> &'a DeclCommandNode {
        match &self.store.commands[self.id.index()].payload {
            CommandNodePayload::Decl(command) => command,
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Builtin(_)
            | CommandNodePayload::Binary(_)
            | CommandNodePayload::Function(_)
            | CommandNodePayload::AnonymousFunction(_)
            | CommandNodePayload::Compound(_) => unreachable!("decl view requires decl payload"),
        }
    }
}

/// Borrowed view of an arena-native binary command payload.
#[derive(Debug, Clone, Copy)]
pub struct BinaryCommandView<'a> {
    store: &'a AstStore,
    id: CommandId,
}

impl<'a> BinaryCommandView<'a> {
    /// Returns the left-hand statement sequence.
    pub fn left(self) -> StmtSeqView<'a> {
        self.store.stmt_seq(self.node().left)
    }

    /// Returns the left-hand statement sequence ID.
    pub fn left_id(self) -> StmtSeqId {
        self.node().left
    }

    /// Returns the binary operator.
    pub fn op(self) -> crate::BinaryOp {
        self.node().op
    }

    /// Returns the source span of the operator token.
    pub fn op_span(self) -> Span {
        self.node().op_span
    }

    /// Returns the right-hand statement sequence.
    pub fn right(self) -> StmtSeqView<'a> {
        self.store.stmt_seq(self.node().right)
    }

    /// Returns the right-hand statement sequence ID.
    pub fn right_id(self) -> StmtSeqId {
        self.node().right
    }

    fn node(self) -> &'a BinaryCommandNode {
        match &self.store.commands[self.id.index()].payload {
            CommandNodePayload::Binary(command) => command,
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Builtin(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Function(_)
            | CommandNodePayload::AnonymousFunction(_)
            | CommandNodePayload::Compound(_) => {
                unreachable!("binary view requires binary payload")
            }
        }
    }
}

/// Borrowed view of an arena-native function command payload.
#[derive(Debug, Clone, Copy)]
pub struct FunctionCommandView<'a> {
    store: &'a AstStore,
    id: CommandId,
}

impl<'a> FunctionCommandView<'a> {
    /// Returns the source span of the `function` keyword when present.
    pub fn function_keyword_span(self) -> Option<Span> {
        self.node().function_keyword_span
    }

    /// Returns parsed function header entries.
    pub fn entries(self) -> &'a [FunctionHeaderEntryNode] {
        self.store
            .function_header_entry_lists
            .get(self.node().entries)
    }

    /// Returns the source span of trailing `()` when present.
    pub fn trailing_parens_span(self) -> Option<Span> {
        self.node().trailing_parens_span
    }

    /// Returns the function body sequence.
    pub fn body(self) -> StmtSeqView<'a> {
        self.store.stmt_seq(self.node().body)
    }

    /// Returns the function body sequence ID.
    pub fn body_id(self) -> StmtSeqId {
        self.node().body
    }

    fn node(self) -> &'a FunctionCommandNode {
        match &self.store.commands[self.id.index()].payload {
            CommandNodePayload::Function(command) => command,
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Builtin(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Binary(_)
            | CommandNodePayload::AnonymousFunction(_)
            | CommandNodePayload::Compound(_) => {
                unreachable!("function view requires function payload")
            }
        }
    }
}

/// Borrowed view of an arena-native anonymous function command payload.
#[derive(Debug, Clone, Copy)]
pub struct AnonymousFunctionCommandView<'a> {
    store: &'a AstStore,
    id: CommandId,
}

impl<'a> AnonymousFunctionCommandView<'a> {
    /// Returns the preserved anonymous function surface.
    pub fn surface(self) -> crate::AnonymousFunctionSurface {
        self.node().surface
    }

    /// Returns the anonymous function body sequence.
    pub fn body(self) -> StmtSeqView<'a> {
        self.store.stmt_seq(self.node().body)
    }

    /// Returns the anonymous function body sequence ID.
    pub fn body_id(self) -> StmtSeqId {
        self.node().body
    }

    /// Returns invocation argument word IDs.
    pub fn arg_ids(self) -> &'a [WordId] {
        self.store.word_id_lists.get(self.node().args)
    }

    /// Returns invocation argument words.
    pub fn args(self) -> impl ExactSizeIterator<Item = WordView<'a>> + 'a {
        self.arg_ids()
            .iter()
            .copied()
            .map(move |id| self.store.word(id))
    }

    fn node(self) -> &'a AnonymousFunctionCommandNode {
        match &self.store.commands[self.id.index()].payload {
            CommandNodePayload::AnonymousFunction(command) => command,
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Builtin(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Binary(_)
            | CommandNodePayload::Function(_)
            | CommandNodePayload::Compound(_) => {
                unreachable!("anonymous function view requires anonymous function payload")
            }
        }
    }
}

/// Borrowed view of an arena-native compound command payload.
#[derive(Debug, Clone, Copy)]
pub struct CompoundCommandView<'a> {
    store: &'a AstStore,
    id: CommandId,
}

impl<'a> CompoundCommandView<'a> {
    /// Returns the compound command payload.
    pub fn node(self) -> &'a CompoundCommandNode {
        match &self.store.commands[self.id.index()].payload {
            CommandNodePayload::Compound(command) => command,
            CommandNodePayload::Simple(_)
            | CommandNodePayload::Builtin(_)
            | CommandNodePayload::Decl(_)
            | CommandNodePayload::Binary(_)
            | CommandNodePayload::Function(_)
            | CommandNodePayload::AnonymousFunction(_) => {
                unreachable!("compound view requires compound payload")
            }
        }
    }
}

/// Borrowed view of a word node.
#[derive(Debug, Clone, Copy)]
pub struct WordView<'a> {
    store: &'a AstStore,
    id: WordId,
}

impl<'a> WordView<'a> {
    /// Returns this node's ID.
    pub fn id(self) -> WordId {
        self.id
    }

    /// Returns this word's parts.
    pub fn parts(self) -> &'a [WordPartArenaNode] {
        self.store.word_part_lists.get(self.node().parts)
    }

    /// Returns this word's source span.
    pub fn span(self) -> Span {
        self.node().span
    }

    /// Returns this word's brace-syntax facts.
    pub fn brace_syntax(self) -> &'a [crate::BraceSyntax] {
        self.store.brace_syntax_lists.get(self.node().brace_syntax)
    }

    fn node(self) -> &'a WordNode {
        &self.store.words[self.id.index()]
    }
}

#[derive(Default)]
struct AstStoreBuilder {
    store: AstStore,
}

impl AstStoreBuilder {
    fn finish(self) -> AstStore {
        self.store
    }

    fn lower_file(&mut self, file: &File) -> FileId {
        let body = self.lower_stmt_seq(&file.body);
        let id = Idx::new(self.store.files.len());
        self.store.files.push(FileNode {
            body,
            span: file.span,
        });
        id
    }

    fn lower_file_body(&mut self, body: StmtSeq, span: Span) -> FileId {
        let body = self.lower_stmt_seq_owned(body);
        let id = Idx::new(self.store.files.len());
        self.store.files.push(FileNode { body, span });
        id
    }

    fn lower_stmt_seq(&mut self, sequence: &StmtSeq) -> StmtSeqId {
        let leading_comments = self
            .store
            .comment_lists
            .push_many(sequence.leading_comments.iter().copied());
        let stmts = sequence
            .stmts
            .iter()
            .map(|stmt| self.lower_stmt(stmt))
            .collect::<Vec<_>>();
        let stmts = self.store.stmt_id_lists.push_many(stmts);
        let trailing_comments = self
            .store
            .comment_lists
            .push_many(sequence.trailing_comments.iter().copied());

        let id = Idx::new(self.store.stmt_seqs.len());
        self.store.stmt_seqs.push(StmtSeqNode {
            leading_comments,
            stmts,
            trailing_comments,
            span: sequence.span,
        });
        id
    }

    fn lower_stmt_seq_owned(&mut self, sequence: StmtSeq) -> StmtSeqId {
        let leading_comments = self
            .store
            .comment_lists
            .push_many(sequence.leading_comments);
        let stmts = sequence
            .stmts
            .into_iter()
            .map(|stmt| self.lower_stmt_owned(stmt))
            .collect::<Vec<_>>();
        let stmts = self.store.stmt_id_lists.push_many(stmts);
        let trailing_comments = self
            .store
            .comment_lists
            .push_many(sequence.trailing_comments);

        let id = Idx::new(self.store.stmt_seqs.len());
        self.store.stmt_seqs.push(StmtSeqNode {
            leading_comments,
            stmts,
            trailing_comments,
            span: sequence.span,
        });
        id
    }

    fn lower_stmt(&mut self, stmt: &Stmt) -> StmtId {
        let leading_comments = self
            .store
            .comment_lists
            .push_many(stmt.leading_comments.iter().copied());
        let command = self.lower_command(&stmt.command);
        let (redirects, redirect_words, redirect_child_sequences) =
            self.lower_redirects(stmt.redirects.iter());

        let id = Idx::new(self.store.stmts.len());
        self.store.stmts.push(StmtNode {
            leading_comments,
            command,
            negated: stmt.negated,
            redirects,
            redirect_words,
            redirect_child_sequences,
            terminator: stmt.terminator,
            terminator_span: stmt.terminator_span,
            inline_comment: stmt.inline_comment,
            span: stmt.span,
        });
        id
    }

    fn lower_stmt_owned(&mut self, stmt: Stmt) -> StmtId {
        let leading_comments = self.store.comment_lists.push_many(stmt.leading_comments);
        let command = self.lower_command_owned(stmt.command);
        let (redirects, redirect_words, redirect_child_sequences) =
            self.lower_redirects(stmt.redirects.iter());

        let id = Idx::new(self.store.stmts.len());
        self.store.stmts.push(StmtNode {
            leading_comments,
            command,
            negated: stmt.negated,
            redirects,
            redirect_words,
            redirect_child_sequences,
            terminator: stmt.terminator,
            terminator_span: stmt.terminator_span,
            inline_comment: stmt.inline_comment,
            span: stmt.span,
        });
        id
    }

    fn lower_command(&mut self, command: &Command) -> CommandId {
        let mut words = Vec::new();
        let mut child_sequences = Vec::new();
        let payload = self.collect_command_children(command, &mut words, &mut child_sequences);

        let words = self.store.word_id_lists.push_many(words);
        let child_sequences = self.store.stmt_seq_id_lists.push_many(child_sequences);
        let id = Idx::new(self.store.commands.len());
        self.store.commands.push(CommandNode {
            kind: command_kind(command),
            span: command_span(command),
            words,
            child_sequences,
            payload,
        });
        id
    }

    fn lower_command_owned(&mut self, command: Command) -> CommandId {
        let mut words = Vec::new();
        let mut child_sequences = Vec::new();
        let payload = self.collect_command_children(&command, &mut words, &mut child_sequences);

        let words = self.store.word_id_lists.push_many(words);
        let child_sequences = self.store.stmt_seq_id_lists.push_many(child_sequences);
        let id = Idx::new(self.store.commands.len());
        self.store.commands.push(CommandNode {
            kind: command_kind(&command),
            span: command_span(&command),
            words,
            child_sequences,
            payload,
        });
        id
    }

    fn lower_word(
        &mut self,
        word: &Word,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> WordId {
        let parts = self.lower_word_parts(word.parts.as_slice(), words, child_sequences);
        let brace_syntax = self
            .store
            .brace_syntax_lists
            .push_many(word.brace_syntax.iter().copied());
        let id = Idx::new(self.store.words.len());
        self.store.words.push(WordNode {
            parts,
            span: word.span,
            brace_syntax,
        });
        id
    }

    fn collect_word_id(
        &mut self,
        word: &Word,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> WordId {
        let id = self.lower_word(word, words, child_sequences);
        words.push(id);
        id
    }

    fn collect_command_children(
        &mut self,
        command: &Command,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> CommandNodePayload {
        match command {
            Command::Simple(command) => {
                let name = self.collect_word_id(&command.name, words, child_sequences);
                let args = command
                    .args
                    .iter()
                    .map(|word| self.collect_word_id(word, words, child_sequences))
                    .collect::<Vec<_>>();
                let args = self.store.word_id_lists.push_many(args);
                let assignments =
                    self.lower_assignments(command.assignments.iter(), words, child_sequences);
                CommandNodePayload::Simple(SimpleCommandNode {
                    name,
                    args,
                    assignments,
                })
            }
            Command::Builtin(command) => {
                self.collect_builtin_children(command, words, child_sequences)
            }
            Command::Decl(command) => {
                let operands = command
                    .operands
                    .iter()
                    .map(|operand| self.lower_decl_operand(operand, words, child_sequences))
                    .collect::<Vec<_>>();
                let operands = self.store.decl_operand_lists.push_many(operands);
                let assignments =
                    self.lower_assignments(command.assignments.iter(), words, child_sequences);
                CommandNodePayload::Decl(DeclCommandNode {
                    variant: command.variant.clone(),
                    variant_span: command.variant_span,
                    operands,
                    assignments,
                })
            }
            Command::Binary(command) => {
                CommandNodePayload::Binary(self.collect_binary_children(command, child_sequences))
            }
            Command::Compound(command) => CommandNodePayload::Compound(
                self.collect_compound_children(command, words, child_sequences),
            ),
            Command::Function(function) => CommandNodePayload::Function(
                self.collect_function_children(function, words, child_sequences),
            ),
            Command::AnonymousFunction(function) => CommandNodePayload::AnonymousFunction(
                self.collect_anonymous_function_children(function, words, child_sequences),
            ),
        }
    }

    fn collect_builtin_children(
        &mut self,
        command: &BuiltinCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> CommandNodePayload {
        match command {
            BuiltinCommand::Break(command) => self.collect_builtin_payload(
                BuiltinCommandNodeKind::Break,
                command.depth.as_ref(),
                &command.extra_args,
                command.assignments.iter(),
                words,
                child_sequences,
            ),
            BuiltinCommand::Continue(command) => self.collect_builtin_payload(
                BuiltinCommandNodeKind::Continue,
                command.depth.as_ref(),
                &command.extra_args,
                command.assignments.iter(),
                words,
                child_sequences,
            ),
            BuiltinCommand::Return(command) => self.collect_builtin_payload(
                BuiltinCommandNodeKind::Return,
                command.code.as_ref(),
                &command.extra_args,
                command.assignments.iter(),
                words,
                child_sequences,
            ),
            BuiltinCommand::Exit(command) => self.collect_builtin_payload(
                BuiltinCommandNodeKind::Exit,
                command.code.as_ref(),
                &command.extra_args,
                command.assignments.iter(),
                words,
                child_sequences,
            ),
        }
    }

    fn collect_builtin_payload<'a>(
        &mut self,
        kind: BuiltinCommandNodeKind,
        primary: Option<&Word>,
        extra_args: &[Word],
        assignments: impl Iterator<Item = &'a Assignment> + Clone,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> CommandNodePayload {
        let primary = primary.map(|word| self.collect_word_id(word, words, child_sequences));
        let extra_args = extra_args
            .iter()
            .map(|word| self.collect_word_id(word, words, child_sequences))
            .collect::<Vec<_>>();
        let extra_args = self.store.word_id_lists.push_many(extra_args);
        let assignments = self.lower_assignments(assignments, words, child_sequences);
        CommandNodePayload::Builtin(BuiltinCommandNode {
            kind,
            primary,
            extra_args,
            assignments,
        })
    }

    fn collect_binary_children(
        &mut self,
        command: &BinaryCommand,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> BinaryCommandNode {
        let left = self.lower_stmt_seq(&StmtSeq {
            leading_comments: Vec::new(),
            stmts: vec![(*command.left).clone()],
            trailing_comments: Vec::new(),
            span: command.left.span,
        });
        let right = self.lower_stmt_seq(&StmtSeq {
            leading_comments: Vec::new(),
            stmts: vec![(*command.right).clone()],
            trailing_comments: Vec::new(),
            span: command.right.span,
        });
        child_sequences.push(left);
        child_sequences.push(right);
        BinaryCommandNode {
            left,
            op: command.op,
            op_span: command.op_span,
            right,
        }
    }

    fn collect_compound_children(
        &mut self,
        command: &CompoundCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> CompoundCommandNode {
        match command {
            CompoundCommand::If(command) => {
                let condition = self.lower_stmt_seq(&command.condition);
                let then_branch = self.lower_stmt_seq(&command.then_branch);
                child_sequences.push(condition);
                child_sequences.push(then_branch);
                let elif_branches = command
                    .elif_branches
                    .iter()
                    .map(|(condition, body)| {
                        let condition = self.lower_stmt_seq(condition);
                        let body = self.lower_stmt_seq(body);
                        child_sequences.push(condition);
                        child_sequences.push(body);
                        ElifBranchNode { condition, body }
                    })
                    .collect::<Vec<_>>();
                let elif_branches = self.store.elif_branch_lists.push_many(elif_branches);
                let else_branch = command.else_branch.as_ref().map(|else_branch| {
                    let id = self.lower_stmt_seq(else_branch);
                    child_sequences.push(id);
                    id
                });
                CompoundCommandNode::If {
                    condition,
                    then_branch,
                    elif_branches,
                    else_branch,
                    syntax: command.syntax,
                }
            }
            CompoundCommand::For(command) => {
                self.collect_for_children(command, words, child_sequences)
            }
            CompoundCommand::Repeat(command) => {
                self.collect_repeat_children(command, words, child_sequences)
            }
            CompoundCommand::Foreach(command) => {
                let words_range = command
                    .words
                    .iter()
                    .map(|word| self.collect_word_id(word, words, child_sequences))
                    .collect::<Vec<_>>();
                let words_range = self.store.word_id_lists.push_many(words_range);
                let body = self.lower_stmt_seq(&command.body);
                child_sequences.push(body);
                CompoundCommandNode::Foreach {
                    variable: command.variable.clone(),
                    variable_span: command.variable_span,
                    words: words_range,
                    body,
                    syntax: command.syntax,
                }
            }
            CompoundCommand::ArithmeticFor(command) => {
                self.collect_arithmetic_for_children(command, words, child_sequences)
            }
            CompoundCommand::While(command) => {
                let condition = self.lower_stmt_seq(&command.condition);
                let body = self.lower_stmt_seq(&command.body);
                child_sequences.push(condition);
                child_sequences.push(body);
                CompoundCommandNode::While { condition, body }
            }
            CompoundCommand::Until(command) => {
                let condition = self.lower_stmt_seq(&command.condition);
                let body = self.lower_stmt_seq(&command.body);
                child_sequences.push(condition);
                child_sequences.push(body);
                CompoundCommandNode::Until { condition, body }
            }
            CompoundCommand::Case(command) => {
                self.collect_case_children(command, words, child_sequences)
            }
            CompoundCommand::Select(command) => {
                self.collect_select_children(command, words, child_sequences)
            }
            CompoundCommand::Subshell(sequence) => {
                let body = self.lower_stmt_seq(sequence);
                child_sequences.push(body);
                CompoundCommandNode::Subshell(body)
            }
            CompoundCommand::BraceGroup(sequence) => {
                let body = self.lower_stmt_seq(sequence);
                child_sequences.push(body);
                CompoundCommandNode::BraceGroup(body)
            }
            CompoundCommand::Arithmetic(command) => {
                self.collect_arithmetic_command_children(command, words, child_sequences)
            }
            CompoundCommand::Conditional(command) => {
                self.collect_conditional_command_children(command, words, child_sequences)
            }
            CompoundCommand::Time(command) => {
                let body = command.command.as_ref().map(|command| {
                    let id = self.lower_stmt_seq(&StmtSeq {
                        leading_comments: Vec::new(),
                        stmts: vec![(**command).clone()],
                        trailing_comments: Vec::new(),
                        span: command.span,
                    });
                    child_sequences.push(id);
                    id
                });
                CompoundCommandNode::Time {
                    posix_format: command.posix_format,
                    command: body,
                }
            }
            CompoundCommand::Coproc(command) => {
                let body = self.lower_stmt_seq(&StmtSeq {
                    leading_comments: Vec::new(),
                    stmts: vec![(*command.body).clone()],
                    trailing_comments: Vec::new(),
                    span: command.body.span,
                });
                child_sequences.push(body);
                CompoundCommandNode::Coproc {
                    name: command.name.clone(),
                    name_span: command.name_span,
                    body,
                }
            }
            CompoundCommand::Always(command) => {
                let body = self.lower_stmt_seq(&command.body);
                let always_body = self.lower_stmt_seq(&command.always_body);
                child_sequences.push(body);
                child_sequences.push(always_body);
                CompoundCommandNode::Always { body, always_body }
            }
        }
    }

    fn collect_for_children(
        &mut self,
        command: &ForCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> CompoundCommandNode {
        let targets = command
            .targets
            .iter()
            .map(|target| ForTargetNode {
                word: self.collect_word_id(&target.word, words, child_sequences),
                name: target.name.clone(),
                span: target.span,
            })
            .collect::<Vec<_>>();
        let targets = self.store.for_target_lists.push_many(targets);
        let header_words = command.words.as_ref().map(|header_words| {
            let header_words = header_words
                .iter()
                .map(|word| self.collect_word_id(word, words, child_sequences))
                .collect::<Vec<_>>();
            self.store.word_id_lists.push_many(header_words)
        });
        let body = self.lower_stmt_seq(&command.body);
        child_sequences.push(body);
        CompoundCommandNode::For {
            targets,
            words: header_words,
            body,
            syntax: command.syntax,
        }
    }

    fn collect_repeat_children(
        &mut self,
        command: &RepeatCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> CompoundCommandNode {
        let count = self.collect_word_id(&command.count, words, child_sequences);
        let body = self.lower_stmt_seq(&command.body);
        child_sequences.push(body);
        CompoundCommandNode::Repeat {
            count,
            body,
            syntax: command.syntax,
        }
    }

    fn collect_arithmetic_for_children(
        &mut self,
        command: &ArithmeticForCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> CompoundCommandNode {
        let body = self.lower_stmt_seq(&command.body);
        child_sequences.push(body);
        CompoundCommandNode::ArithmeticFor(Box::new(ArithmeticForCommandNode {
            left_paren_span: command.left_paren_span,
            init_span: command.init_span,
            init_ast: self.lower_arithmetic_expr_option(
                command.init_ast.as_ref(),
                words,
                child_sequences,
            ),
            first_semicolon_span: command.first_semicolon_span,
            condition_span: command.condition_span,
            condition_ast: self.lower_arithmetic_expr_option(
                command.condition_ast.as_ref(),
                words,
                child_sequences,
            ),
            second_semicolon_span: command.second_semicolon_span,
            step_span: command.step_span,
            step_ast: self.lower_arithmetic_expr_option(
                command.step_ast.as_ref(),
                words,
                child_sequences,
            ),
            right_paren_span: command.right_paren_span,
            body,
        }))
    }

    fn collect_case_children(
        &mut self,
        command: &CaseCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> CompoundCommandNode {
        let word = self.collect_word_id(&command.word, words, child_sequences);
        let cases = command
            .cases
            .iter()
            .map(|case| {
                let patterns = case
                    .patterns
                    .iter()
                    .map(|pattern| self.lower_pattern(pattern, words, child_sequences))
                    .collect::<Vec<_>>();
                let patterns = self.store.pattern_lists.push_many(patterns);
                let body = self.lower_stmt_seq(&case.body);
                child_sequences.push(body);
                CaseItemNode {
                    patterns,
                    body,
                    terminator: case.terminator,
                    terminator_span: case.terminator_span,
                }
            })
            .collect::<Vec<_>>();
        let cases = self.store.case_item_lists.push_many(cases);
        CompoundCommandNode::Case { word, cases }
    }

    fn collect_select_children(
        &mut self,
        command: &SelectCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> CompoundCommandNode {
        let header_words = command
            .words
            .iter()
            .map(|word| self.collect_word_id(word, words, child_sequences))
            .collect::<Vec<_>>();
        let header_words = self.store.word_id_lists.push_many(header_words);
        let body = self.lower_stmt_seq(&command.body);
        child_sequences.push(body);
        CompoundCommandNode::Select {
            variable: command.variable.clone(),
            variable_span: command.variable_span,
            words: header_words,
            body,
        }
    }

    fn collect_arithmetic_command_children(
        &mut self,
        command: &ArithmeticCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> CompoundCommandNode {
        CompoundCommandNode::Arithmetic(ArithmeticCommandNode {
            left_paren_span: command.left_paren_span,
            expr_span: command.expr_span,
            expr_ast: self.lower_arithmetic_expr_option(
                command.expr_ast.as_ref(),
                words,
                child_sequences,
            ),
            right_paren_span: command.right_paren_span,
        })
    }

    fn collect_conditional_command_children(
        &mut self,
        command: &ConditionalCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> CompoundCommandNode {
        CompoundCommandNode::Conditional(ConditionalCommandNode {
            expression: self.lower_conditional_expr(&command.expression, words, child_sequences),
            left_bracket_span: command.left_bracket_span,
            right_bracket_span: command.right_bracket_span,
        })
    }

    fn collect_function_children(
        &mut self,
        function: &FunctionDef,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> FunctionCommandNode {
        let entries = function
            .header
            .entries
            .iter()
            .map(|entry| FunctionHeaderEntryNode {
                word: self.collect_word_id(&entry.word, words, child_sequences),
                static_name: entry.static_name.clone(),
            })
            .collect::<Vec<_>>();
        let entries = self.store.function_header_entry_lists.push_many(entries);
        let body = self.lower_stmt_seq(&StmtSeq {
            leading_comments: Vec::new(),
            stmts: vec![(*function.body).clone()],
            trailing_comments: Vec::new(),
            span: function.body.span,
        });
        child_sequences.push(body);
        FunctionCommandNode {
            function_keyword_span: function.header.function_keyword_span,
            entries,
            trailing_parens_span: function.header.trailing_parens_span,
            body,
        }
    }

    fn collect_anonymous_function_children(
        &mut self,
        function: &AnonymousFunctionCommand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> AnonymousFunctionCommandNode {
        let body = self.lower_stmt_seq(&StmtSeq {
            leading_comments: Vec::new(),
            stmts: vec![(*function.body).clone()],
            trailing_comments: Vec::new(),
            span: function.body.span,
        });
        child_sequences.push(body);
        let args = function
            .args
            .iter()
            .map(|word| self.collect_word_id(word, words, child_sequences))
            .collect::<Vec<_>>();
        let args = self.store.word_id_lists.push_many(args);
        AnonymousFunctionCommandNode {
            surface: function.surface,
            body,
            args,
        }
    }

    fn lower_decl_operand(
        &mut self,
        operand: &DeclOperand,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> DeclOperandNode {
        match operand {
            DeclOperand::Flag(word) => {
                DeclOperandNode::Flag(self.collect_word_id(word, words, child_sequences))
            }
            DeclOperand::Name(reference) => {
                DeclOperandNode::Name(self.lower_var_ref(reference, words, child_sequences))
            }
            DeclOperand::Assignment(assignment) => DeclOperandNode::Assignment(
                self.lower_assignment(assignment, words, child_sequences),
            ),
            DeclOperand::Dynamic(word) => {
                DeclOperandNode::Dynamic(self.collect_word_id(word, words, child_sequences))
            }
        }
    }

    fn lower_assignments<'a>(
        &mut self,
        assignments: impl Iterator<Item = &'a Assignment>,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> IdRange<AssignmentNode> {
        let assignments = assignments
            .map(|assignment| self.lower_assignment(assignment, words, child_sequences))
            .collect::<Vec<_>>();
        self.store.assignment_lists.push_many(assignments)
    }

    fn lower_assignment(
        &mut self,
        assignment: &Assignment,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> AssignmentNode {
        AssignmentNode {
            target: self.lower_var_ref(&assignment.target, words, child_sequences),
            value: match &assignment.value {
                AssignmentValue::Scalar(word) => {
                    AssignmentValueNode::Scalar(self.collect_word_id(word, words, child_sequences))
                }
                AssignmentValue::Compound(array) => AssignmentValueNode::Compound(
                    self.lower_array_expr(array, words, child_sequences),
                ),
            },
            append: assignment.append,
            span: assignment.span,
        }
    }

    fn lower_var_ref(
        &mut self,
        reference: &crate::VarRef,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> VarRefNode {
        VarRefNode {
            name: reference.name.clone(),
            name_span: reference.name_span,
            subscript: reference
                .subscript
                .as_deref()
                .map(|subscript| Box::new(self.lower_subscript(subscript, words, child_sequences))),
            span: reference.span,
        }
    }

    fn lower_subscript(
        &mut self,
        subscript: &Subscript,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> SubscriptNode {
        SubscriptNode {
            text: subscript.text.clone(),
            raw: subscript.raw.clone(),
            kind: subscript.kind,
            interpretation: subscript.interpretation,
            word_ast: subscript
                .word_ast
                .as_ref()
                .map(|word| self.collect_word_id(word, words, child_sequences)),
            arithmetic_ast: self.lower_arithmetic_expr_option(
                subscript.arithmetic_ast.as_ref(),
                words,
                child_sequences,
            ),
        }
    }

    fn lower_array_expr(
        &mut self,
        array: &crate::ArrayExpr,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> ArrayExprNode {
        let elements = array
            .elements
            .iter()
            .map(|element| self.lower_array_elem(element, words, child_sequences))
            .collect::<Vec<_>>();
        let elements = self.store.array_elem_lists.push_many(elements);
        ArrayExprNode {
            kind: array.kind,
            elements,
            span: array.span,
        }
    }

    fn lower_array_elem(
        &mut self,
        element: &crate::ArrayElem,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> ArrayElemNode {
        match element {
            crate::ArrayElem::Sequential(value) => ArrayElemNode::Sequential(
                self.lower_array_value_word(value, words, child_sequences),
            ),
            crate::ArrayElem::Keyed { key, value } => ArrayElemNode::Keyed {
                key: self.lower_subscript(key, words, child_sequences),
                value: self.lower_array_value_word(value, words, child_sequences),
            },
            crate::ArrayElem::KeyedAppend { key, value } => ArrayElemNode::KeyedAppend {
                key: self.lower_subscript(key, words, child_sequences),
                value: self.lower_array_value_word(value, words, child_sequences),
            },
        }
    }

    fn lower_array_value_word(
        &mut self,
        value: &crate::ArrayValueWord,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> ArrayValueWordNode {
        ArrayValueWordNode {
            word: self.collect_word_id(&value.word, words, child_sequences),
            has_top_level_unquoted_comma: value.has_top_level_unquoted_comma,
        }
    }

    fn lower_word_parts(
        &mut self,
        parts: &[WordPartNode],
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> IdRange<WordPartArenaNode> {
        let parts = parts
            .iter()
            .map(|part| self.lower_word_part(part, words, child_sequences))
            .collect::<Vec<_>>();
        self.store.word_part_lists.push_many(parts)
    }

    fn lower_word_part(
        &mut self,
        part: &WordPartNode,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> WordPartArenaNode {
        let kind = match &part.kind {
            WordPart::Literal(text) => WordPartArena::Literal(text.clone()),
            WordPart::ZshQualifiedGlob(glob) => WordPartArena::ZshQualifiedGlob(
                self.lower_zsh_qualified_glob(glob, words, child_sequences),
            ),
            WordPart::SingleQuoted { value, dollar } => WordPartArena::SingleQuoted {
                value: value.clone(),
                dollar: *dollar,
            },
            WordPart::DoubleQuoted { parts, dollar } => WordPartArena::DoubleQuoted {
                parts: self.lower_word_parts(parts, words, child_sequences),
                dollar: *dollar,
            },
            WordPart::Variable(name) => WordPartArena::Variable(name.clone()),
            WordPart::CommandSubstitution { body, syntax } => {
                let body = self.lower_stmt_seq(body);
                child_sequences.push(body);
                WordPartArena::CommandSubstitution {
                    body,
                    syntax: *syntax,
                }
            }
            WordPart::ArithmeticExpansion {
                expression,
                expression_ast,
                expression_word_ast,
                syntax,
            } => WordPartArena::ArithmeticExpansion {
                expression: expression.clone(),
                expression_ast: self.lower_arithmetic_expr_option(
                    expression_ast.as_ref(),
                    words,
                    child_sequences,
                ),
                expression_word_ast: self.collect_word_id(
                    expression_word_ast,
                    words,
                    child_sequences,
                ),
                syntax: *syntax,
            },
            WordPart::Parameter(expansion) => WordPartArena::Parameter(
                self.lower_parameter_expansion(expansion, words, child_sequences),
            ),
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                colon_variant,
            } => WordPartArena::ParameterExpansion {
                reference: self.lower_var_ref(reference, words, child_sequences),
                operator: operator.clone(),
                operand: operand.clone(),
                operand_word_ast: operand_word_ast
                    .as_ref()
                    .map(|word| self.collect_word_id(word, words, child_sequences)),
                colon_variant: *colon_variant,
            },
            WordPart::Length(reference) => {
                WordPartArena::Length(self.lower_var_ref(reference, words, child_sequences))
            }
            WordPart::ArrayAccess(reference) => {
                WordPartArena::ArrayAccess(self.lower_var_ref(reference, words, child_sequences))
            }
            WordPart::ArrayLength(reference) => {
                WordPartArena::ArrayLength(self.lower_var_ref(reference, words, child_sequences))
            }
            WordPart::ArrayIndices(reference) => {
                WordPartArena::ArrayIndices(self.lower_var_ref(reference, words, child_sequences))
            }
            WordPart::Substring {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
            } => WordPartArena::Substring {
                reference: self.lower_var_ref(reference, words, child_sequences),
                offset: offset.clone(),
                offset_ast: self.lower_arithmetic_expr_option(
                    offset_ast.as_ref(),
                    words,
                    child_sequences,
                ),
                offset_word_ast: self.collect_word_id(offset_word_ast, words, child_sequences),
                length: length.clone(),
                length_ast: self.lower_arithmetic_expr_option(
                    length_ast.as_ref(),
                    words,
                    child_sequences,
                ),
                length_word_ast: length_word_ast
                    .as_ref()
                    .map(|word| self.collect_word_id(word, words, child_sequences)),
            },
            WordPart::ArraySlice {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
            } => WordPartArena::ArraySlice {
                reference: self.lower_var_ref(reference, words, child_sequences),
                offset: offset.clone(),
                offset_ast: self.lower_arithmetic_expr_option(
                    offset_ast.as_ref(),
                    words,
                    child_sequences,
                ),
                offset_word_ast: self.collect_word_id(offset_word_ast, words, child_sequences),
                length: length.clone(),
                length_ast: self.lower_arithmetic_expr_option(
                    length_ast.as_ref(),
                    words,
                    child_sequences,
                ),
                length_word_ast: length_word_ast
                    .as_ref()
                    .map(|word| self.collect_word_id(word, words, child_sequences)),
            },
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                colon_variant,
            } => WordPartArena::IndirectExpansion {
                reference: self.lower_var_ref(reference, words, child_sequences),
                operator: operator.clone(),
                operand: operand.clone(),
                operand_word_ast: operand_word_ast
                    .as_ref()
                    .map(|word| self.collect_word_id(word, words, child_sequences)),
                colon_variant: *colon_variant,
            },
            WordPart::PrefixMatch { prefix, kind } => WordPartArena::PrefixMatch {
                prefix: prefix.clone(),
                kind: *kind,
            },
            WordPart::ProcessSubstitution { body, is_input } => {
                let body = self.lower_stmt_seq(body);
                child_sequences.push(body);
                WordPartArena::ProcessSubstitution {
                    body,
                    is_input: *is_input,
                }
            }
            WordPart::Transformation {
                reference,
                operator,
            } => WordPartArena::Transformation {
                reference: self.lower_var_ref(reference, words, child_sequences),
                operator: *operator,
            },
        };
        WordPartArenaNode {
            kind,
            span: part.span,
        }
    }

    fn lower_redirects<'a>(
        &mut self,
        redirects: impl Iterator<Item = &'a Redirect>,
    ) -> (IdRange<RedirectNode>, IdRange<WordId>, IdRange<StmtSeqId>) {
        let mut words = Vec::new();
        let mut child_sequences = Vec::new();
        let redirects = redirects
            .map(|redirect| self.lower_redirect(redirect, &mut words, &mut child_sequences))
            .collect::<Vec<_>>();
        (
            self.store.redirect_lists.push_many(redirects),
            self.store.word_id_lists.push_many(words),
            self.store.stmt_seq_id_lists.push_many(child_sequences),
        )
    }

    fn lower_redirect(
        &mut self,
        redirect: &Redirect,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> RedirectNode {
        let target = self.lower_redirect_target(&redirect.target, words, child_sequences);
        RedirectNode {
            fd: redirect.fd,
            fd_var: redirect.fd_var.clone(),
            fd_var_span: redirect.fd_var_span,
            kind: redirect.kind,
            span: redirect.span,
            target,
        }
    }

    fn lower_redirect_target(
        &mut self,
        target: &RedirectTarget,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> RedirectTargetNode {
        match target {
            RedirectTarget::Word(word) => {
                RedirectTargetNode::Word(self.collect_word_id(word, words, child_sequences))
            }
            RedirectTarget::Heredoc(heredoc) => {
                let raw = self.collect_word_id(&heredoc.delimiter.raw, words, child_sequences);
                let parts = heredoc
                    .body
                    .parts
                    .iter()
                    .map(|part| self.lower_heredoc_body_part(part, words, child_sequences))
                    .collect::<Vec<_>>();
                let parts = self.store.heredoc_body_part_lists.push_many(parts);
                RedirectTargetNode::Heredoc(HeredocNode {
                    delimiter: HeredocDelimiterNode {
                        raw,
                        cooked: heredoc.delimiter.cooked.clone(),
                        span: heredoc.delimiter.span,
                        quoted: heredoc.delimiter.quoted,
                        expands_body: heredoc.delimiter.expands_body,
                        strip_tabs: heredoc.delimiter.strip_tabs,
                    },
                    body: HeredocBodyNode {
                        mode: heredoc.body.mode,
                        source_backed: heredoc.body.source_backed,
                        parts,
                        span: heredoc.body.span,
                    },
                })
            }
        }
    }

    fn lower_heredoc_body_part(
        &mut self,
        part: &crate::HeredocBodyPartNode,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> ArenaHeredocBodyPartNode {
        let kind = match &part.kind {
            HeredocBodyPart::Literal(text) => ArenaHeredocBodyPart::Literal(text.clone()),
            HeredocBodyPart::Variable(name) => ArenaHeredocBodyPart::Variable(name.clone()),
            HeredocBodyPart::CommandSubstitution { body, syntax } => {
                let body = self.lower_stmt_seq(body);
                child_sequences.push(body);
                ArenaHeredocBodyPart::CommandSubstitution {
                    body,
                    syntax: *syntax,
                }
            }
            HeredocBodyPart::ArithmeticExpansion {
                expression,
                expression_ast,
                expression_word_ast,
                syntax,
            } => ArenaHeredocBodyPart::ArithmeticExpansion {
                expression: expression.clone(),
                expression_ast: self.lower_arithmetic_expr_option(
                    expression_ast.as_ref(),
                    words,
                    child_sequences,
                ),
                expression_word_ast: self.collect_word_id(
                    expression_word_ast,
                    words,
                    child_sequences,
                ),
                syntax: *syntax,
            },
            HeredocBodyPart::Parameter(expansion) => ArenaHeredocBodyPart::Parameter(Box::new(
                self.lower_parameter_expansion(expansion, words, child_sequences),
            )),
        };
        ArenaHeredocBodyPartNode {
            kind,
            span: part.span,
        }
    }

    fn lower_zsh_qualified_glob(
        &mut self,
        glob: &crate::ZshQualifiedGlob,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> ZshQualifiedGlobNode {
        let segments = glob
            .segments
            .iter()
            .map(|segment| match segment {
                crate::ZshGlobSegment::Pattern(pattern) => {
                    ZshGlobSegmentNode::Pattern(self.lower_pattern(pattern, words, child_sequences))
                }
                crate::ZshGlobSegment::InlineControl(control) => {
                    ZshGlobSegmentNode::InlineControl(*control)
                }
            })
            .collect::<Vec<_>>();
        let segments = self.store.zsh_glob_segment_lists.push_many(segments);
        let qualifiers = glob.qualifiers.as_ref().map(|qualifiers| {
            let fragments = self
                .store
                .zsh_glob_qualifier_lists
                .push_many(qualifiers.fragments.iter().cloned());
            ZshGlobQualifierGroupNode {
                span: qualifiers.span,
                kind: qualifiers.kind,
                fragments,
            }
        });
        ZshQualifiedGlobNode {
            span: glob.span,
            segments,
            qualifiers,
        }
    }

    fn lower_parameter_expansion(
        &mut self,
        expansion: &crate::ParameterExpansion,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> ParameterExpansionNode {
        ParameterExpansionNode {
            syntax: match &expansion.syntax {
                crate::ParameterExpansionSyntax::Bourne(expansion) => {
                    ParameterExpansionSyntaxNode::Bourne(self.lower_bourne_parameter_expansion(
                        expansion,
                        words,
                        child_sequences,
                    ))
                }
                crate::ParameterExpansionSyntax::Zsh(expansion) => {
                    ParameterExpansionSyntaxNode::Zsh(self.lower_zsh_parameter_expansion(
                        expansion,
                        words,
                        child_sequences,
                    ))
                }
            },
            span: expansion.span,
            raw_body: expansion.raw_body.clone(),
        }
    }

    fn lower_bourne_parameter_expansion(
        &mut self,
        expansion: &crate::BourneParameterExpansion,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> BourneParameterExpansionNode {
        match expansion {
            crate::BourneParameterExpansion::Access { reference } => {
                BourneParameterExpansionNode::Access {
                    reference: self.lower_var_ref(reference, words, child_sequences),
                }
            }
            crate::BourneParameterExpansion::Length { reference } => {
                BourneParameterExpansionNode::Length {
                    reference: self.lower_var_ref(reference, words, child_sequences),
                }
            }
            crate::BourneParameterExpansion::Indices { reference } => {
                BourneParameterExpansionNode::Indices {
                    reference: self.lower_var_ref(reference, words, child_sequences),
                }
            }
            crate::BourneParameterExpansion::Indirect {
                reference,
                operator,
                operand,
                operand_word_ast,
                colon_variant,
            } => BourneParameterExpansionNode::Indirect {
                reference: self.lower_var_ref(reference, words, child_sequences),
                operator: operator.clone(),
                operand: operand.clone(),
                operand_word_ast: operand_word_ast
                    .as_ref()
                    .map(|word| self.collect_word_id(word, words, child_sequences)),
                colon_variant: *colon_variant,
            },
            crate::BourneParameterExpansion::PrefixMatch { prefix, kind } => {
                BourneParameterExpansionNode::PrefixMatch {
                    prefix: prefix.clone(),
                    kind: *kind,
                }
            }
            crate::BourneParameterExpansion::Slice {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
            } => BourneParameterExpansionNode::Slice {
                reference: self.lower_var_ref(reference, words, child_sequences),
                offset: offset.clone(),
                offset_ast: self.lower_arithmetic_expr_option(
                    offset_ast.as_ref(),
                    words,
                    child_sequences,
                ),
                offset_word_ast: self.collect_word_id(offset_word_ast, words, child_sequences),
                length: length.clone(),
                length_ast: self.lower_arithmetic_expr_option(
                    length_ast.as_ref(),
                    words,
                    child_sequences,
                ),
                length_word_ast: length_word_ast
                    .as_ref()
                    .map(|word| self.collect_word_id(word, words, child_sequences)),
            },
            crate::BourneParameterExpansion::Operation {
                reference,
                operator,
                operand,
                operand_word_ast,
                colon_variant,
            } => BourneParameterExpansionNode::Operation {
                reference: self.lower_var_ref(reference, words, child_sequences),
                operator: operator.clone(),
                operand: operand.clone(),
                operand_word_ast: operand_word_ast
                    .as_ref()
                    .map(|word| self.collect_word_id(word, words, child_sequences)),
                colon_variant: *colon_variant,
            },
            crate::BourneParameterExpansion::Transformation {
                reference,
                operator,
            } => BourneParameterExpansionNode::Transformation {
                reference: self.lower_var_ref(reference, words, child_sequences),
                operator: *operator,
            },
        }
    }

    fn lower_zsh_parameter_expansion(
        &mut self,
        expansion: &crate::ZshParameterExpansion,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> ZshParameterExpansionNode {
        let modifiers = expansion
            .modifiers
            .iter()
            .map(|modifier| self.lower_zsh_modifier(modifier, words, child_sequences))
            .collect::<Vec<_>>();
        let modifiers = self.store.zsh_modifier_lists.push_many(modifiers);
        ZshParameterExpansionNode {
            target: self.lower_zsh_expansion_target(&expansion.target, words, child_sequences),
            modifiers,
            length_prefix: expansion.length_prefix,
            operation: expansion.operation.as_ref().map(|operation| {
                self.lower_zsh_expansion_operation(operation, words, child_sequences)
            }),
        }
    }

    fn lower_zsh_expansion_target(
        &mut self,
        target: &crate::ZshExpansionTarget,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> ZshExpansionTargetNode {
        match target {
            crate::ZshExpansionTarget::Reference(reference) => ZshExpansionTargetNode::Reference(
                self.lower_var_ref(reference, words, child_sequences),
            ),
            crate::ZshExpansionTarget::Nested(expansion) => ZshExpansionTargetNode::Nested(
                Box::new(self.lower_parameter_expansion(expansion, words, child_sequences)),
            ),
            crate::ZshExpansionTarget::Word(word) => {
                ZshExpansionTargetNode::Word(self.collect_word_id(word, words, child_sequences))
            }
            crate::ZshExpansionTarget::Empty => ZshExpansionTargetNode::Empty,
        }
    }

    fn lower_zsh_modifier(
        &mut self,
        modifier: &crate::ZshModifier,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> ZshModifierNode {
        ZshModifierNode {
            name: modifier.name,
            argument: modifier.argument.clone(),
            argument_word_ast: modifier
                .argument_word_ast
                .as_ref()
                .map(|word| self.collect_word_id(word, words, child_sequences)),
            argument_delimiter: modifier.argument_delimiter,
            span: modifier.span,
        }
    }

    fn lower_zsh_expansion_operation(
        &mut self,
        operation: &crate::ZshExpansionOperation,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> ZshExpansionOperationNode {
        match operation {
            crate::ZshExpansionOperation::PatternOperation {
                kind,
                operand,
                operand_word_ast,
            } => ZshExpansionOperationNode::PatternOperation {
                kind: *kind,
                operand: operand.clone(),
                operand_word_ast: self.collect_word_id(operand_word_ast, words, child_sequences),
            },
            crate::ZshExpansionOperation::Defaulting {
                kind,
                operand,
                operand_word_ast,
                colon_variant,
            } => ZshExpansionOperationNode::Defaulting {
                kind: *kind,
                operand: operand.clone(),
                operand_word_ast: self.collect_word_id(operand_word_ast, words, child_sequences),
                colon_variant: *colon_variant,
            },
            crate::ZshExpansionOperation::TrimOperation {
                kind,
                operand,
                operand_word_ast,
            } => ZshExpansionOperationNode::TrimOperation {
                kind: *kind,
                operand: operand.clone(),
                operand_word_ast: self.collect_word_id(operand_word_ast, words, child_sequences),
            },
            crate::ZshExpansionOperation::ReplacementOperation {
                kind,
                pattern,
                pattern_word_ast,
                replacement,
                replacement_word_ast,
            } => ZshExpansionOperationNode::ReplacementOperation {
                kind: *kind,
                pattern: pattern.clone(),
                pattern_word_ast: self.collect_word_id(pattern_word_ast, words, child_sequences),
                replacement: replacement.clone(),
                replacement_word_ast: replacement_word_ast
                    .as_ref()
                    .map(|word| self.collect_word_id(word, words, child_sequences)),
            },
            crate::ZshExpansionOperation::Slice {
                offset,
                offset_word_ast,
                length,
                length_word_ast,
            } => ZshExpansionOperationNode::Slice {
                offset: offset.clone(),
                offset_word_ast: self.collect_word_id(offset_word_ast, words, child_sequences),
                length: length.clone(),
                length_word_ast: length_word_ast
                    .as_ref()
                    .map(|word| self.collect_word_id(word, words, child_sequences)),
            },
            crate::ZshExpansionOperation::Unknown { text, word_ast } => {
                ZshExpansionOperationNode::Unknown {
                    text: text.clone(),
                    word_ast: self.collect_word_id(word_ast, words, child_sequences),
                }
            }
        }
    }

    fn lower_pattern(
        &mut self,
        pattern: &Pattern,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> PatternNode {
        let parts = pattern
            .parts
            .iter()
            .map(|part| self.lower_pattern_part(part, words, child_sequences))
            .collect::<Vec<_>>();
        let parts = self.store.pattern_part_lists.push_many(parts);
        PatternNode {
            parts,
            span: pattern.span,
        }
    }

    fn lower_pattern_part(
        &mut self,
        part: &crate::PatternPartNode,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> PatternPartArenaNode {
        let kind = match &part.kind {
            PatternPart::Literal(text) => PatternPartArena::Literal(text.clone()),
            PatternPart::AnyString => PatternPartArena::AnyString,
            PatternPart::AnyChar => PatternPartArena::AnyChar,
            PatternPart::CharClass(text) => PatternPartArena::CharClass(text.clone()),
            PatternPart::Group { kind, patterns } => {
                let patterns = patterns
                    .iter()
                    .map(|pattern| self.lower_pattern(pattern, words, child_sequences))
                    .collect::<Vec<_>>();
                let patterns = self.store.pattern_lists.push_many(patterns);
                PatternPartArena::Group {
                    kind: *kind,
                    patterns,
                }
            }
            PatternPart::Word(word) => {
                PatternPartArena::Word(self.collect_word_id(word, words, child_sequences))
            }
        };
        PatternPartArenaNode {
            kind,
            span: part.span,
        }
    }

    fn lower_conditional_expr(
        &mut self,
        expression: &ConditionalExpr,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> ConditionalExprArena {
        match expression {
            ConditionalExpr::Binary(expr) => ConditionalExprArena::Binary {
                left: Box::new(self.lower_conditional_expr(&expr.left, words, child_sequences)),
                op: expr.op,
                op_span: expr.op_span,
                right: Box::new(self.lower_conditional_expr(&expr.right, words, child_sequences)),
            },
            ConditionalExpr::Unary(expr) => ConditionalExprArena::Unary {
                op: expr.op,
                op_span: expr.op_span,
                expr: Box::new(self.lower_conditional_expr(&expr.expr, words, child_sequences)),
            },
            ConditionalExpr::Parenthesized(expr) => ConditionalExprArena::Parenthesized {
                left_paren_span: expr.left_paren_span,
                expr: Box::new(self.lower_conditional_expr(&expr.expr, words, child_sequences)),
                right_paren_span: expr.right_paren_span,
            },
            ConditionalExpr::Word(word) => {
                ConditionalExprArena::Word(self.collect_word_id(word, words, child_sequences))
            }
            ConditionalExpr::Pattern(pattern) => {
                ConditionalExprArena::Pattern(self.lower_pattern(pattern, words, child_sequences))
            }
            ConditionalExpr::Regex(word) => {
                ConditionalExprArena::Regex(self.collect_word_id(word, words, child_sequences))
            }
            ConditionalExpr::VarRef(var_ref) => {
                ConditionalExprArena::VarRef(self.lower_var_ref(var_ref, words, child_sequences))
            }
        }
    }

    fn lower_arithmetic_expr_option(
        &mut self,
        expression: Option<&ArithmeticExprNode>,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> Option<ArithmeticExprArenaNode> {
        expression.map(|expression| self.lower_arithmetic_expr(expression, words, child_sequences))
    }

    fn lower_arithmetic_expr(
        &mut self,
        expression: &ArithmeticExprNode,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> ArithmeticExprArenaNode {
        ArithmeticExprArenaNode {
            kind: match &expression.kind {
                ArithmeticExpr::Number(text) => ArithmeticExprArena::Number(text.clone()),
                ArithmeticExpr::Variable(name) => ArithmeticExprArena::Variable(name.clone()),
                ArithmeticExpr::Indexed { name, index } => ArithmeticExprArena::Indexed {
                    name: name.clone(),
                    index: Box::new(self.lower_arithmetic_expr(index, words, child_sequences)),
                },
                ArithmeticExpr::ShellWord(word) => ArithmeticExprArena::ShellWord(
                    self.collect_word_id(word, words, child_sequences),
                ),
                ArithmeticExpr::Parenthesized { expression } => {
                    ArithmeticExprArena::Parenthesized {
                        expression: Box::new(self.lower_arithmetic_expr(
                            expression,
                            words,
                            child_sequences,
                        )),
                    }
                }
                ArithmeticExpr::Unary { op, expr } => ArithmeticExprArena::Unary {
                    op: *op,
                    expr: Box::new(self.lower_arithmetic_expr(expr, words, child_sequences)),
                },
                ArithmeticExpr::Postfix { expr, op } => ArithmeticExprArena::Postfix {
                    expr: Box::new(self.lower_arithmetic_expr(expr, words, child_sequences)),
                    op: *op,
                },
                ArithmeticExpr::Binary { left, op, right } => ArithmeticExprArena::Binary {
                    left: Box::new(self.lower_arithmetic_expr(left, words, child_sequences)),
                    op: *op,
                    right: Box::new(self.lower_arithmetic_expr(right, words, child_sequences)),
                },
                ArithmeticExpr::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => ArithmeticExprArena::Conditional {
                    condition: Box::new(self.lower_arithmetic_expr(
                        condition,
                        words,
                        child_sequences,
                    )),
                    then_expr: Box::new(self.lower_arithmetic_expr(
                        then_expr,
                        words,
                        child_sequences,
                    )),
                    else_expr: Box::new(self.lower_arithmetic_expr(
                        else_expr,
                        words,
                        child_sequences,
                    )),
                },
                ArithmeticExpr::Assignment { target, op, value } => {
                    ArithmeticExprArena::Assignment {
                        target: self.lower_arithmetic_lvalue(target, words, child_sequences),
                        op: *op,
                        value: Box::new(self.lower_arithmetic_expr(value, words, child_sequences)),
                    }
                }
            },
            span: expression.span,
        }
    }

    fn lower_arithmetic_lvalue(
        &mut self,
        lvalue: &ArithmeticLvalue,
        words: &mut Vec<WordId>,
        child_sequences: &mut Vec<StmtSeqId>,
    ) -> ArithmeticLvalueArena {
        match lvalue {
            ArithmeticLvalue::Variable(name) => ArithmeticLvalueArena::Variable(name.clone()),
            ArithmeticLvalue::Indexed { name, index } => ArithmeticLvalueArena::Indexed {
                name: name.clone(),
                index: Box::new(self.lower_arithmetic_expr(index, words, child_sequences)),
            },
        }
    }
}

fn command_kind(command: &Command) -> ArenaFileCommandKind {
    match command {
        Command::Simple(_) => ArenaFileCommandKind::Simple,
        Command::Builtin(_) => ArenaFileCommandKind::Builtin,
        Command::Decl(_) => ArenaFileCommandKind::Decl,
        Command::Binary(_) => ArenaFileCommandKind::Binary,
        Command::Compound(_) => ArenaFileCommandKind::Compound,
        Command::Function(_) => ArenaFileCommandKind::Function,
        Command::AnonymousFunction(_) => ArenaFileCommandKind::AnonymousFunction,
    }
}

fn command_span(command: &Command) -> Span {
    match command {
        Command::Simple(command) => command.span,
        Command::Builtin(BuiltinCommand::Break(command)) => command.span,
        Command::Builtin(BuiltinCommand::Continue(command)) => command.span,
        Command::Builtin(BuiltinCommand::Return(command)) => command.span,
        Command::Builtin(BuiltinCommand::Exit(command)) => command.span,
        Command::Decl(command) => command.span,
        Command::Binary(command) => command.span,
        Command::Compound(CompoundCommand::If(command)) => command.span,
        Command::Compound(CompoundCommand::For(command)) => command.span,
        Command::Compound(CompoundCommand::Repeat(command)) => command.span,
        Command::Compound(CompoundCommand::Foreach(command)) => command.span,
        Command::Compound(CompoundCommand::ArithmeticFor(command)) => command.span,
        Command::Compound(CompoundCommand::While(command)) => command.span,
        Command::Compound(CompoundCommand::Until(command)) => command.span,
        Command::Compound(CompoundCommand::Case(command)) => command.span,
        Command::Compound(CompoundCommand::Select(command)) => command.span,
        Command::Compound(CompoundCommand::Subshell(sequence))
        | Command::Compound(CompoundCommand::BraceGroup(sequence)) => sequence.span,
        Command::Compound(CompoundCommand::Arithmetic(command)) => command.span,
        Command::Compound(CompoundCommand::Time(command)) => command.span,
        Command::Compound(CompoundCommand::Conditional(command)) => command.span,
        Command::Compound(CompoundCommand::Coproc(command)) => command.span,
        Command::Compound(CompoundCommand::Always(command)) => command.span,
        Command::Function(command) => command.span,
        Command::AnonymousFunction(command) => command.span,
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ArenaFile, ArithmeticCommand, ArithmeticExpr, ArithmeticExprNode, ArrayElem, ArrayExpr,
        ArrayKind, ArrayValueWord, Assignment, AssignmentValue, BourneParameterExpansion,
        BuiltinCommand, BuiltinCommandNodeKind, Command, CommandSubstitutionSyntax,
        CompoundCommand, ConditionalCommand, ConditionalExpr, DeclClause, DeclOperand, Heredoc,
        HeredocBody, HeredocBodyMode, HeredocBodyPart, HeredocBodyPartNode, HeredocDelimiter, Name,
        ParameterExpansion, ParameterExpansionSyntax, Pattern, PatternPart, PatternPartNode,
        Redirect, RedirectKind, RedirectTarget, SimpleCommand, Span, Stmt, StmtSeq, Subscript,
        SubscriptInterpretation, SubscriptKind, Word, WordPart, WordPartNode, ZshExpansionTarget,
        ZshGlobSegment, ZshParameterExpansion, ZshQualifiedGlob,
    };

    #[test]
    fn arena_file_round_trips_simple_sequence() {
        let file = crate::File {
            body: StmtSeq {
                leading_comments: Vec::new(),
                stmts: vec![Stmt {
                    leading_comments: Vec::new(),
                    command: Command::Simple(crate::SimpleCommand {
                        name: Word::literal("echo"),
                        args: vec![Word::literal("hello")],
                        assignments: Box::new([]),
                        span: Span::new(),
                    }),
                    negated: false,
                    redirects: Box::new([]),
                    terminator: None,
                    terminator_span: None,
                    inline_comment: None,
                    span: Span::new(),
                }],
                trailing_comments: Vec::new(),
                span: Span::new(),
            },
            span: Span::new(),
        };

        let arena = ArenaFile::from_file(&file);

        assert_eq!(arena.store.file_count(), 1);
        assert_eq!(arena.store.stmt_seq_count(), 1);
        assert_eq!(arena.store.stmt_count(), 1);
        assert_eq!(arena.store.command_count(), 1);
        assert_eq!(arena.view().body().stmt_ids().len(), 1);
        assert_eq!(arena.view().body().stmts().len(), 1);

        let materialized = arena.to_file();
        assert_eq!(materialized.body.len(), 1);
        let Command::Simple(command) = &materialized.body[0].command else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 1);
    }

    #[test]
    fn arena_file_builds_from_owned_body() {
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("echo"),
            args: vec![Word::literal("hello")],
            assignments: Box::new([]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_body(file.body, file.span);
        let materialized = arena.to_file();

        assert_eq!(arena.store.file_count(), 1);
        assert_eq!(arena.store.stmt_seq_count(), 1);
        assert_eq!(materialized.body.len(), 1);
    }

    #[test]
    fn command_substitution_body_is_reachable_from_command() {
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("echo"),
            args: vec![word_with_command_substitution()],
            assignments: Box::new([]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.child_sequence_ids().len(), 1);
        assert!(command.word_ids().len() >= 2);
    }

    #[test]
    fn simple_command_payload_is_arena_native() {
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("printf"),
            args: vec![Word::literal("%s"), Word::literal("value")],
            assignments: Box::new([Assignment {
                target: crate::VarRef {
                    name: Name::new("LC_ALL"),
                    name_span: Span::new(),
                    subscript: None,
                    span: Span::new(),
                },
                value: AssignmentValue::Scalar(Word::literal("C")),
                append: false,
                span: Span::new(),
            }]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();
        let simple = command.simple().expect("expected native simple payload");

        assert_eq!(simple.name().parts().len(), 1);
        assert_eq!(simple.arg_ids().len(), 2);
        assert_eq!(simple.args().len(), 2);
        assert_eq!(simple.assignments().len(), 1);

        let materialized = arena.to_file();
        let Command::Simple(command) = &materialized.body[0].command else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 2);
        assert_eq!(command.assignments.len(), 1);
    }

    #[test]
    fn builtin_command_payload_is_arena_native() {
        let file = file_with_command(Command::Builtin(BuiltinCommand::Return(
            crate::ReturnCommand {
                code: Some(Word::literal("7")),
                extra_args: vec![Word::literal("extra")],
                assignments: Box::new([Assignment {
                    target: crate::VarRef {
                        name: Name::new("status"),
                        name_span: Span::new(),
                        subscript: None,
                        span: Span::new(),
                    },
                    value: AssignmentValue::Scalar(Word::literal("set")),
                    append: false,
                    span: Span::new(),
                }]),
                span: Span::new(),
            },
        )));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();
        let builtin = command.builtin().expect("expected native builtin payload");

        assert_eq!(builtin.kind(), BuiltinCommandNodeKind::Return);
        assert!(builtin.primary().is_some());
        assert_eq!(builtin.extra_arg_ids().len(), 1);
        assert_eq!(builtin.extra_args().len(), 1);
        assert_eq!(builtin.assignments().len(), 1);

        let materialized = arena.to_file();
        let Command::Builtin(BuiltinCommand::Return(command)) = &materialized.body[0].command
        else {
            panic!("expected return command");
        };
        assert!(command.code.is_some());
        assert_eq!(command.extra_args.len(), 1);
        assert_eq!(command.assignments.len(), 1);
    }

    #[test]
    fn declaration_command_payload_is_arena_native() {
        let file = file_with_command(Command::Decl(DeclClause {
            variant: Name::new("declare"),
            variant_span: Span::new(),
            operands: vec![
                DeclOperand::Flag(Word::literal("-a")),
                DeclOperand::Name(var_ref_with_dynamic_subscript("arr")),
            ],
            assignments: Box::new([Assignment {
                target: crate::VarRef {
                    name: Name::new("prefix"),
                    name_span: Span::new(),
                    subscript: None,
                    span: Span::new(),
                },
                value: AssignmentValue::Scalar(Word::literal("value")),
                append: false,
                span: Span::new(),
            }]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();
        let decl = command.decl().expect("expected native declaration payload");

        assert_eq!(decl.variant(), "declare");
        assert_eq!(decl.operands().len(), 2);
        assert_eq!(decl.assignments().len(), 1);
        assert_eq!(command.child_sequence_ids().len(), 1);

        let materialized = arena.to_file();
        let Command::Decl(command) = &materialized.body[0].command else {
            panic!("expected declaration command");
        };
        assert_eq!(command.operands.len(), 2);
        assert_eq!(command.assignments.len(), 1);
    }

    #[test]
    fn binary_command_payload_is_arena_native() {
        let left = Stmt {
            leading_comments: Vec::new(),
            command: Command::Simple(SimpleCommand {
                name: Word::literal("left"),
                args: Vec::new(),
                assignments: Box::new([]),
                span: Span::new(),
            }),
            negated: false,
            redirects: Box::new([]),
            terminator: None,
            terminator_span: None,
            inline_comment: None,
            span: Span::new(),
        };
        let right = Stmt {
            leading_comments: Vec::new(),
            command: Command::Simple(SimpleCommand {
                name: Word::literal("right"),
                args: Vec::new(),
                assignments: Box::new([]),
                span: Span::new(),
            }),
            negated: false,
            redirects: Box::new([]),
            terminator: None,
            terminator_span: None,
            inline_comment: None,
            span: Span::new(),
        };
        let file = file_with_command(Command::Binary(crate::BinaryCommand {
            left: Box::new(left),
            op: crate::BinaryOp::And,
            op_span: Span::new(),
            right: Box::new(right),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();
        let binary = command.binary().expect("expected native binary payload");

        assert_eq!(binary.op(), crate::BinaryOp::And);
        assert_eq!(binary.left().stmt_ids().len(), 1);
        assert_eq!(binary.right().stmt_ids().len(), 1);
        assert_eq!(command.child_sequence_ids().len(), 2);

        let materialized = arena.to_file();
        let Command::Binary(command) = &materialized.body[0].command else {
            panic!("expected binary command");
        };
        assert_eq!(command.op, crate::BinaryOp::And);
    }

    #[test]
    fn function_payloads_are_arena_native() {
        let body = Box::new(Stmt {
            leading_comments: Vec::new(),
            command: Command::Simple(SimpleCommand {
                name: Word::literal("body"),
                args: Vec::new(),
                assignments: Box::new([]),
                span: Span::new(),
            }),
            negated: false,
            redirects: Box::new([]),
            terminator: None,
            terminator_span: None,
            inline_comment: None,
            span: Span::new(),
        });
        let function_file = file_with_command(Command::Function(crate::FunctionDef {
            header: crate::FunctionHeader {
                function_keyword_span: Some(Span::new()),
                entries: vec![crate::FunctionHeaderEntry {
                    word: Word::literal("fn"),
                    static_name: Some(Name::new("fn")),
                }],
                trailing_parens_span: Some(Span::new()),
            },
            body: body.clone(),
            span: Span::new(),
        }));
        let anonymous_file = file_with_command(Command::AnonymousFunction(
            crate::AnonymousFunctionCommand {
                surface: crate::AnonymousFunctionSurface::Parens {
                    parens_span: Span::new(),
                },
                body,
                args: vec![Word::literal("arg")],
                span: Span::new(),
            },
        ));

        let arena = ArenaFile::from_file(&function_file);
        let command = arena.view().body().stmts().next().unwrap().command();
        let function = command
            .function()
            .expect("expected native function payload");
        assert_eq!(function.entries().len(), 1);
        assert_eq!(function.body().stmt_ids().len(), 1);
        assert_eq!(command.child_sequence_ids().len(), 1);
        let Command::Function(materialized) = &arena.to_file().body[0].command else {
            panic!("expected function command");
        };
        assert_eq!(materialized.header.entries.len(), 1);

        let arena = ArenaFile::from_file(&anonymous_file);
        let command = arena.view().body().stmts().next().unwrap().command();
        let function = command
            .anonymous_function()
            .expect("expected native anonymous function payload");
        assert_eq!(function.arg_ids().len(), 1);
        assert_eq!(function.body().stmt_ids().len(), 1);
        assert_eq!(command.child_sequence_ids().len(), 1);
        let Command::AnonymousFunction(materialized) = &arena.to_file().body[0].command else {
            panic!("expected anonymous function command");
        };
        assert_eq!(materialized.args.len(), 1);
    }

    #[test]
    fn compound_payloads_are_arena_native() {
        let if_file = file_with_command(Command::Compound(CompoundCommand::If(crate::IfCommand {
            condition: simple_sequence("test"),
            then_branch: simple_sequence("then"),
            elif_branches: vec![(simple_sequence("elif"), simple_sequence("elif_body"))],
            else_branch: Some(simple_sequence("else")),
            syntax: crate::IfSyntax::ThenFi {
                then_span: Span::new(),
                fi_span: Span::new(),
            },
            span: Span::new(),
        })));
        let case_file = file_with_command(Command::Compound(CompoundCommand::Case(
            crate::CaseCommand {
                word: Word::literal("value"),
                cases: vec![crate::CaseItem {
                    patterns: vec![Pattern {
                        parts: vec![PatternPartNode::new(
                            PatternPart::Word(Word::literal("pattern")),
                            Span::new(),
                        )],
                        span: Span::new(),
                    }],
                    body: simple_sequence("case_body"),
                    terminator: crate::CaseTerminator::Break,
                    terminator_span: Some(Span::new()),
                }],
                span: Span::new(),
            },
        )));

        let arena = ArenaFile::from_file(&if_file);
        let command = arena.view().body().stmts().next().unwrap().command();
        let compound = command
            .compound()
            .expect("expected native compound payload");
        let crate::CompoundCommandNode::If {
            elif_branches,
            else_branch,
            ..
        } = compound.node()
        else {
            panic!("expected if payload");
        };
        assert_eq!(elif_branches.len(), 1);
        assert!(else_branch.is_some());
        assert_eq!(command.child_sequence_ids().len(), 5);
        let Command::Compound(CompoundCommand::If(command)) = &arena.to_file().body[0].command
        else {
            panic!("expected if command");
        };
        assert_eq!(command.elif_branches.len(), 1);
        assert!(command.else_branch.is_some());

        let arena = ArenaFile::from_file(&case_file);
        let command = arena.view().body().stmts().next().unwrap().command();
        let compound = command
            .compound()
            .expect("expected native compound payload");
        let crate::CompoundCommandNode::Case { cases, .. } = compound.node() else {
            panic!("expected case payload");
        };
        assert_eq!(cases.len(), 1);
        assert_eq!(command.child_sequence_ids().len(), 1);
        let Command::Compound(CompoundCommand::Case(command)) = &arena.to_file().body[0].command
        else {
            panic!("expected case command");
        };
        assert_eq!(command.cases.len(), 1);
    }

    #[test]
    fn heredoc_substitution_body_is_reachable_from_statement_redirects() {
        let redirect = Redirect {
            fd: None,
            fd_var: None,
            fd_var_span: None,
            kind: RedirectKind::HereDoc,
            span: Span::new(),
            target: RedirectTarget::Heredoc(Heredoc {
                delimiter: HeredocDelimiter {
                    raw: Word::literal("EOF"),
                    cooked: "EOF".to_string(),
                    span: Span::new(),
                    quoted: false,
                    expands_body: true,
                    strip_tabs: false,
                },
                body: HeredocBody {
                    mode: HeredocBodyMode::Expanding,
                    source_backed: false,
                    parts: vec![HeredocBodyPartNode::new(
                        HeredocBodyPart::CommandSubstitution {
                            body: simple_sequence("inner"),
                            syntax: CommandSubstitutionSyntax::DollarParen,
                        },
                        Span::new(),
                    )],
                    span: Span::new(),
                },
            }),
        };
        let file = file_with_stmt(Stmt {
            leading_comments: Vec::new(),
            command: Command::Simple(SimpleCommand {
                name: Word::literal("cat"),
                args: Vec::new(),
                assignments: Box::new([]),
                span: Span::new(),
            }),
            negated: false,
            redirects: Box::new([redirect]),
            terminator: None,
            terminator_span: None,
            inline_comment: None,
            span: Span::new(),
        });

        let arena = ArenaFile::from_file(&file);
        let stmt = arena.view().body().stmts().next().unwrap();

        assert_eq!(stmt.redirect_child_sequence_ids().len(), 1);
        assert!(!stmt.redirect_word_ids().is_empty());
    }

    #[test]
    fn zsh_qualified_glob_pattern_words_are_command_words() {
        let pattern = Pattern {
            parts: vec![PatternPartNode::new(
                PatternPart::Word(Word::literal("nested")),
                Span::new(),
            )],
            span: Span::new(),
        };
        let glob_word = Word {
            parts: vec![WordPartNode::new(
                WordPart::ZshQualifiedGlob(ZshQualifiedGlob {
                    span: Span::new(),
                    segments: vec![ZshGlobSegment::Pattern(pattern)],
                    qualifiers: None,
                }),
                Span::new(),
            )],
            span: Span::new(),
            brace_syntax: Vec::new(),
        };
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("print"),
            args: vec![glob_word],
            assignments: Box::new([]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.word_ids().len(), 3);
    }

    #[test]
    fn conditional_and_arithmetic_words_link_nested_substitutions() {
        let conditional_file = file_with_command(Command::Compound(CompoundCommand::Conditional(
            ConditionalCommand {
                expression: ConditionalExpr::Word(word_with_command_substitution()),
                span: Span::new(),
                left_bracket_span: Span::new(),
                right_bracket_span: Span::new(),
            },
        )));
        let arithmetic_file = file_with_command(Command::Compound(CompoundCommand::Arithmetic(
            ArithmeticCommand {
                span: Span::new(),
                left_paren_span: Span::new(),
                expr_span: Some(Span::new()),
                expr_ast: Some(ArithmeticExprNode::new(
                    ArithmeticExpr::ShellWord(word_with_command_substitution()),
                    Span::new(),
                )),
                right_paren_span: Span::new(),
            },
        )));

        for file in [conditional_file, arithmetic_file] {
            let arena = ArenaFile::from_file(&file);
            let command = arena.view().body().stmts().next().unwrap().command();

            assert_eq!(command.child_sequence_ids().len(), 1);
            assert!(!command.word_ids().is_empty());
        }
    }

    #[test]
    fn keyed_array_subscript_words_are_assignment_words() {
        let assignment = Assignment {
            target: crate::VarRef {
                name: Name::new("arr"),
                name_span: Span::new(),
                subscript: None,
                span: Span::new(),
            },
            value: AssignmentValue::Compound(ArrayExpr {
                kind: ArrayKind::Associative,
                elements: vec![ArrayElem::Keyed {
                    key: dynamic_subscript(),
                    value: ArrayValueWord::from(Word::literal("value")),
                }],
                span: Span::new(),
            }),
            append: false,
            span: Span::new(),
        };
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("printf"),
            args: Vec::new(),
            assignments: Box::new([assignment]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.child_sequence_ids().len(), 1);
        assert!(command.word_ids().len() >= 3);
    }

    #[test]
    fn assignment_target_subscript_words_are_assignment_words() {
        let assignment = Assignment {
            target: crate::VarRef {
                name: Name::new("arr"),
                name_span: Span::new(),
                subscript: Some(Box::new(dynamic_subscript())),
                span: Span::new(),
            },
            value: AssignmentValue::Scalar(Word::literal("value")),
            append: false,
            span: Span::new(),
        };
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("printf"),
            args: Vec::new(),
            assignments: Box::new([assignment]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.child_sequence_ids().len(), 1);
        assert!(command.word_ids().len() >= 3);
    }

    #[test]
    fn bourne_parameter_subscript_words_are_command_words() {
        let expansion = ParameterExpansion {
            syntax: ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access {
                reference: var_ref_with_dynamic_subscript("arr"),
            }),
            span: Span::new(),
            raw_body: crate::SourceText::cooked(Span::new(), "arr[$(idx)]"),
        };
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("echo"),
            args: vec![word(vec![WordPart::Parameter(expansion)])],
            assignments: Box::new([]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.child_sequence_ids().len(), 1);
        assert!(command.word_ids().len() >= 3);
    }

    #[test]
    fn declaration_name_subscript_words_are_command_words() {
        let file = file_with_command(Command::Decl(DeclClause {
            variant: Name::new("declare"),
            variant_span: Span::new(),
            operands: vec![DeclOperand::Name(var_ref_with_dynamic_subscript("arr"))],
            assignments: Box::new([]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.child_sequence_ids().len(), 1);
        assert!(!command.word_ids().is_empty());
    }

    #[test]
    fn zsh_reference_target_subscript_words_are_command_words() {
        let expansion = ParameterExpansion {
            syntax: ParameterExpansionSyntax::Zsh(ZshParameterExpansion {
                target: ZshExpansionTarget::Reference(var_ref_with_dynamic_subscript("arr")),
                modifiers: Vec::new(),
                length_prefix: None,
                operation: None,
            }),
            span: Span::new(),
            raw_body: crate::SourceText::cooked(Span::new(), "arr[$(idx)]"),
        };
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("echo"),
            args: vec![word(vec![WordPart::Parameter(expansion)])],
            assignments: Box::new([]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.child_sequence_ids().len(), 1);
        assert!(command.word_ids().len() >= 2);
    }

    #[test]
    fn transformation_subscript_words_are_command_words() {
        let file = file_with_command(Command::Simple(SimpleCommand {
            name: Word::literal("echo"),
            args: vec![word(vec![WordPart::Transformation {
                reference: var_ref_with_dynamic_subscript("arr"),
                operator: 'Q',
            }])],
            assignments: Box::new([]),
            span: Span::new(),
        }));

        let arena = ArenaFile::from_file(&file);
        let command = arena.view().body().stmts().next().unwrap().command();

        assert_eq!(command.child_sequence_ids().len(), 1);
        assert!(command.word_ids().len() >= 2);
    }

    fn file_with_command(command: Command) -> crate::File {
        file_with_stmt(Stmt {
            leading_comments: Vec::new(),
            command,
            negated: false,
            redirects: Box::new([]),
            terminator: None,
            terminator_span: None,
            inline_comment: None,
            span: Span::new(),
        })
    }

    fn file_with_stmt(stmt: Stmt) -> crate::File {
        crate::File {
            body: StmtSeq {
                leading_comments: Vec::new(),
                stmts: vec![stmt],
                trailing_comments: Vec::new(),
                span: Span::new(),
            },
            span: Span::new(),
        }
    }

    fn simple_sequence(name: &str) -> StmtSeq {
        StmtSeq {
            leading_comments: Vec::new(),
            stmts: vec![Stmt {
                leading_comments: Vec::new(),
                command: Command::Simple(SimpleCommand {
                    name: Word::literal(name),
                    args: Vec::new(),
                    assignments: Box::new([]),
                    span: Span::new(),
                }),
                negated: false,
                redirects: Box::new([]),
                terminator: None,
                terminator_span: None,
                inline_comment: None,
                span: Span::new(),
            }],
            trailing_comments: Vec::new(),
            span: Span::new(),
        }
    }

    fn word_with_command_substitution() -> Word {
        word(vec![WordPart::CommandSubstitution {
            body: simple_sequence("subcmd"),
            syntax: CommandSubstitutionSyntax::DollarParen,
        }])
    }

    fn word(parts: Vec<WordPart>) -> Word {
        Word {
            parts: parts
                .into_iter()
                .map(|part| WordPartNode::new(part, Span::new()))
                .collect(),
            span: Span::new(),
            brace_syntax: Vec::new(),
        }
    }

    fn var_ref_with_dynamic_subscript(name: &str) -> crate::VarRef {
        crate::VarRef {
            name: Name::new(name),
            name_span: Span::new(),
            subscript: Some(Box::new(dynamic_subscript())),
            span: Span::new(),
        }
    }

    fn dynamic_subscript() -> Subscript {
        Subscript {
            text: crate::SourceText::cooked(Span::new(), "$(key)"),
            raw: None,
            kind: SubscriptKind::Ordinary,
            interpretation: SubscriptInterpretation::Indexed,
            word_ast: Some(word_with_command_substitution()),
            arithmetic_ast: None,
        }
    }
}
