use rustc_hash::FxHashSet;
use shuck_ast::{Command, ConditionalBinaryOp, ConditionalExpr, ConditionalUnaryOp, Span, Word};
use shuck_semantic::{
    Binding, BindingAttributes, BindingKind, DeclarationBuiltin, DeclarationOperand, Reference,
    ReferenceId,
};

use crate::{Checker, LinterFacts, Rule, Violation, WordQuote};

pub struct QuotedBashSource;

impl Violation for QuotedBashSource {
    fn rule() -> Rule {
        Rule::QuotedBashSource
    }

    fn message(&self) -> String {
        "array references should choose an explicit element or selector".to_owned()
    }
}

pub fn quoted_bash_source(checker: &mut Checker) {
    let semantic = checker.semantic();
    let analysis = semantic.analysis();
    let spans = checker
        .facts()
        .plain_unindexed_reference_spans()
        .iter()
        .copied()
        .filter(|span| {
            plain_reference_is_array_like(
                checker.facts(),
                checker.source(),
                semantic,
                &analysis,
                *span,
            )
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || QuotedBashSource);
}

fn span_is_within(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn plain_reference_is_array_like(
    facts: &LinterFacts<'_>,
    source: &str,
    semantic: &shuck_semantic::SemanticModel,
    analysis: &shuck_semantic::SemanticAnalysis<'_>,
    span: Span,
) -> bool {
    semantic.references().iter().any(|reference| {
        reference.span == span
            && reference_is_array_like(facts, source, semantic, analysis, reference)
    })
}

fn reference_is_array_like(
    facts: &LinterFacts<'_>,
    source: &str,
    semantic: &shuck_semantic::SemanticModel,
    analysis: &shuck_semantic::SemanticAnalysis<'_>,
    reference: &Reference,
) -> bool {
    if semantic.is_guarded_parameter_reference(reference.id)
        || reference_has_prior_dominating_presence_test(
            facts, source, semantic, analysis, reference,
        )
        || reference_reads_into_same_name_array_writer(facts, semantic, reference)
    {
        return false;
    }

    semantic.reference_is_predefined_runtime_array(reference.id)
        || reference_is_unbound_bash_runtime_array(semantic, reference)
        || semantic
            .resolved_binding(reference.id)
            .is_some_and(|binding| {
                semantic.binding_visible_at(binding.id, reference.span)
                    && !semantic.binding_cleared_before(binding.id, reference.span)
                    && !binding_reset_by_name_only_declaration_before(
                        semantic,
                        binding,
                        reference.span,
                    )
                    && (binding_is_array_like(binding)
                        || binding_inherits_indexed_array_type(semantic, binding))
            })
}

fn reference_is_unbound_bash_runtime_array(
    semantic: &shuck_semantic::SemanticModel,
    reference: &Reference,
) -> bool {
    semantic.resolved_binding(reference.id).is_none()
        && is_bash_runtime_array_name(reference.name.as_str())
}

fn binding_is_array_like(binding: &Binding) -> bool {
    let declared_array = binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC);
    (declared_array && !is_uninitialized_local_array_declaration(binding))
        || matches!(
            binding.kind,
            BindingKind::ArrayAssignment | BindingKind::MapfileTarget
        )
}

fn binding_inherits_indexed_array_type(
    semantic: &shuck_semantic::SemanticModel,
    binding: &Binding,
) -> bool {
    if binding_resets_indexed_array_type(binding) {
        return false;
    }

    let prior_bindings = semantic
        .bindings_for(&binding.name)
        .iter()
        .copied()
        .filter(|candidate_id| {
            let candidate = semantic.binding(*candidate_id);
            candidate.span.start.offset < binding.span.start.offset
                && !semantic.binding_cleared_before(*candidate_id, binding.span)
                && !binding_reset_by_name_only_declaration_before(semantic, candidate, binding.span)
        })
        .map(|candidate_id| semantic.binding(candidate_id));

    for candidate in prior_bindings.rev() {
        if binding_resets_indexed_array_type(candidate) {
            return false;
        }
        if binding_is_sticky_indexed_array(candidate) {
            return true;
        }
    }

    false
}

fn binding_resets_indexed_array_type(binding: &Binding) -> bool {
    matches!(
        binding.kind,
        BindingKind::ArithmeticAssignment
            | BindingKind::GetoptsTarget
            | BindingKind::Imported
            | BindingKind::LoopVariable
            | BindingKind::PrintfTarget
    ) || (matches!(binding.kind, BindingKind::ReadTarget)
        && !binding.attributes.contains(BindingAttributes::ARRAY))
        || (matches!(binding.kind, BindingKind::Declaration(_))
            && !binding
                .attributes
                .contains(BindingAttributes::DECLARATION_INITIALIZED))
}

fn binding_is_sticky_indexed_array(binding: &Binding) -> bool {
    !is_uninitialized_local_array_declaration(binding)
        && (binding.attributes.contains(BindingAttributes::ARRAY)
            || matches!(
                binding.kind,
                BindingKind::ArrayAssignment | BindingKind::MapfileTarget
            ))
}

fn is_uninitialized_local_array_declaration(binding: &Binding) -> bool {
    matches!(
        binding.kind,
        BindingKind::Declaration(DeclarationBuiltin::Local)
    ) && binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
        && !binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
}

fn reference_reads_into_same_name_array_writer(
    facts: &LinterFacts<'_>,
    semantic: &shuck_semantic::SemanticModel,
    reference: &Reference,
) -> bool {
    semantic
        .bindings_for(&reference.name)
        .iter()
        .copied()
        .any(|binding_id| {
            let binding = semantic.binding(binding_id);
            binding.span.start.offset <= reference.span.start.offset
                && binding_is_same_name_array_writer(binding)
                && facts
                    .commands()
                    .iter()
                    .filter(|command| matches!(command.command(), Command::Simple(_)))
                    .filter(|command| {
                        let span = command.span();
                        span_is_within(span, binding.span) && span_is_within(span, reference.span)
                    })
                    .min_by_key(|command| command.span().end.offset - command.span().start.offset)
                    .is_some()
        })
}

fn reference_has_prior_dominating_presence_test(
    facts: &LinterFacts<'_>,
    source: &str,
    semantic: &shuck_semantic::SemanticModel,
    analysis: &shuck_semantic::SemanticAnalysis<'_>,
    reference: &Reference,
) -> bool {
    if loop_header_word_quote(facts, reference.span)
        .is_some_and(|quote| quote != WordQuote::Unquoted)
    {
        return false;
    }

    facts.commands().iter().any(|command| {
        command.span().end.offset < reference.span.start.offset
            && presence_test_reference_spans(source, semantic, command, &reference.name)
                .into_iter()
                .any(|test_id| reference_id_dominates_reference(analysis, reference, test_id))
    })
}

fn loop_header_word_quote(facts: &LinterFacts<'_>, span: Span) -> Option<WordQuote> {
    facts
        .for_headers()
        .iter()
        .flat_map(|header| header.words().iter())
        .chain(
            facts
                .select_headers()
                .iter()
                .flat_map(|header| header.words().iter()),
        )
        .find(|word| span_is_within(word.span(), span))
        .map(|word| word.classification().quote)
}

fn presence_test_reference_spans(
    source: &str,
    semantic: &shuck_semantic::SemanticModel,
    command: &crate::CommandFact<'_>,
    name: &shuck_ast::Name,
) -> Vec<ReferenceId> {
    let mut spans = Vec::new();

    if let Some(simple_test) = command.simple_test() {
        for word in simple_test.truthy_expression_words(source) {
            spans.extend(word_reference_ids(semantic, word, name));
        }
        for (_, operand) in simple_test.string_unary_expression_words(source) {
            spans.extend(word_reference_ids(semantic, operand, name));
        }
    }

    if let Some(conditional) = command.conditional() {
        spans.extend(conditional_presence_test_reference_ids(
            semantic,
            conditional.expression(),
            name,
        ));
    }

    spans
}

fn conditional_presence_test_reference_ids(
    semantic: &shuck_semantic::SemanticModel,
    expression: &ConditionalExpr,
    name: &shuck_ast::Name,
) -> Vec<ReferenceId> {
    match expression {
        ConditionalExpr::Word(word) => word_reference_ids(semantic, word, name),
        ConditionalExpr::Unary(unary)
            if matches!(
                unary.op,
                ConditionalUnaryOp::EmptyString | ConditionalUnaryOp::NonEmptyString
            ) =>
        {
            conditional_presence_test_reference_ids(semantic, &unary.expr, name)
        }
        ConditionalExpr::Binary(binary)
            if matches!(
                binary.op,
                ConditionalBinaryOp::And | ConditionalBinaryOp::Or
            ) =>
        {
            let mut spans = conditional_presence_test_reference_ids(semantic, &binary.left, name);
            spans.extend(conditional_presence_test_reference_ids(
                semantic,
                &binary.right,
                name,
            ));
            spans
        }
        ConditionalExpr::Parenthesized(parenthesized) => {
            conditional_presence_test_reference_ids(semantic, &parenthesized.expr, name)
        }
        ConditionalExpr::Unary(_)
        | ConditionalExpr::Binary(_)
        | ConditionalExpr::Pattern(_)
        | ConditionalExpr::Regex(_)
        | ConditionalExpr::VarRef(_) => Vec::new(),
    }
}

fn word_reference_ids(
    semantic: &shuck_semantic::SemanticModel,
    word: &Word,
    name: &shuck_ast::Name,
) -> Vec<ReferenceId> {
    semantic
        .references()
        .iter()
        .filter(|reference| {
            reference.name == *name
                && !matches!(
                    reference.kind,
                    shuck_semantic::ReferenceKind::DeclarationName
                )
                && span_is_within(word.span, reference.span)
        })
        .map(|reference| reference.id)
        .collect()
}

fn reference_id_dominates_reference(
    analysis: &shuck_semantic::SemanticAnalysis<'_>,
    reference: &Reference,
    test_id: ReferenceId,
) -> bool {
    let cfg = analysis.cfg();
    let reference_blocks = cfg
        .blocks()
        .iter()
        .filter(|block| block.references.contains(&reference.id))
        .map(|block| block.id)
        .collect::<FxHashSet<_>>();
    let test_blocks = cfg
        .blocks()
        .iter()
        .filter(|block| block.references.contains(&test_id))
        .map(|block| block.id)
        .collect::<FxHashSet<_>>();
    if reference_blocks.is_empty() || test_blocks.is_empty() {
        return false;
    }

    if !reference_blocks.is_disjoint(&test_blocks) {
        return false;
    }

    let scope_entry = cfg
        .scope_entry(reference.scope)
        .unwrap_or_else(|| cfg.entry());
    let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();
    let mut stack = vec![scope_entry];
    let mut seen = FxHashSet::default();
    while let Some(block_id) = stack.pop() {
        if test_blocks.contains(&block_id)
            || unreachable.contains(&block_id)
            || !seen.insert(block_id)
        {
            continue;
        }
        if reference_blocks.contains(&block_id) {
            return false;
        }
        for (successor, _) in cfg.successors(block_id) {
            stack.push(*successor);
        }
    }

    true
}

fn binding_is_same_name_array_writer(binding: &Binding) -> bool {
    matches!(
        binding.kind,
        BindingKind::ArrayAssignment | BindingKind::MapfileTarget
    ) || (matches!(binding.kind, BindingKind::ReadTarget)
        && binding.attributes.contains(BindingAttributes::ARRAY))
}

fn binding_reset_by_name_only_declaration_before(
    semantic: &shuck_semantic::SemanticModel,
    binding: &Binding,
    at: Span,
) -> bool {
    semantic.declarations().iter().any(|declaration| {
        declaration.span.start.offset > binding.span.start.offset
            && declaration.span.start.offset < at.start.offset
            && semantic.scope_at(declaration.span.start.offset) == binding.scope
            && declaration.operands.iter().any(|operand| {
                matches!(
                    operand,
                    DeclarationOperand::Name { name, .. } if name == &binding.name
                )
            })
    })
}

fn is_bash_runtime_array_name(name: &str) -> bool {
    matches!(
        name,
        "BASH_ALIASES"
            | "BASH_ARGC"
            | "BASH_ARGV"
            | "BASH_CMDS"
            | "BASH_LINENO"
            | "BASH_REMATCH"
            | "BASH_SOURCE"
            | "BASH_VERSINFO"
            | "COMP_WORDS"
            | "COMPREPLY"
            | "COPROC"
            | "DIRSTACK"
            | "FUNCNAME"
            | "GROUPS"
            | "MAPFILE"
            | "PIPESTATUS"
    )
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, lint_file_at_path};
    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn reports_plain_unindexed_array_references() {
        let source = "\
#!/bin/bash
arr=(one two)
declare -A map=([key]=value)
read -ra read_items
mapfile map_items
x=\"$BASH_SOURCE\"
y=\"${BASH_SOURCE}\"
printf '%s\\n' $arr \"${arr}\" pre${arr}post \"$map\" \"$read_items\" \"$map_items\"
source \"$(dirname \"$BASH_SOURCE\")/helper.bash\"
if [[ \"$BASH_SOURCE\" == foo ]]; then :; fi
for item in \"$BASH_SOURCE\"; do
  :
done
cat <<EOF
$arr
${arr}
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$BASH_SOURCE",
                "${BASH_SOURCE}",
                "$arr",
                "${arr}",
                "${arr}",
                "$map",
                "$read_items",
                "$map_items",
                "$BASH_SOURCE",
                "$BASH_SOURCE",
                "$BASH_SOURCE",
                "$arr",
                "${arr}",
            ]
        );
    }

    #[test]
    fn ignores_scalar_indexed_selector_and_non_access_forms() {
        let source = "\
#!/bin/bash
name=scalar
MAPFILE=scalar
arr=(one two)
x=$BASH_SOURCE
y=${BASH_SOURCE}
z=\"${BASH_SOURCE[0]}\"
q=\"${BASH_SOURCE[@]}\"
r=\"${BASH_SOURCE[*]}\"
s=\"${BASH_SOURCE%/*}\"
t=\"${BASH_SOURCE:-fallback}\"
v=\"${BASH_SOURCE-}\"
u=\"\\$BASH_SOURCE\"
printf '%s\\n' \"$name\" \"${arr[0]}\" \"${arr[@]}\" \"${arr[*]}\" \"${arr%one}\" \"${arr:-fallback}\"
only_declared() {
  local -a local_array
  printf '%s\\n' \"$local_array\"
}
for item in \"$@\"; do
  item=($item)
done
read -ra read_items <<<\"$read_items\"
printf '%s\\n' \"$MAPFILE\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$BASH_SOURCE", "${BASH_SOURCE}"]
        );
    }

    #[test]
    fn ignores_follow_up_loop_headers_after_presence_guard() {
        let source = "\
#!/bin/bash
filelist=()
filelist+=(\"$1\")
if [ -z \"${filelist[*]}\" ]; then
  exit
fi
for item in $filelist; do
  :
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn string_binary_conditions_do_not_count_as_presence_guards() {
        let source = "\
#!/bin/bash
apt_pkgs=()
for pkg in \"$@\"; do
  pkg=(one two three)
  if [[ \"${pkg[0]}\" == one ]]; then
    :
  fi
  if hasPackage \"$pkg\"; then
    apt_pkgs+=(\"$pkg\")
  fi
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$pkg", "$pkg"]
        );
    }

    #[test]
    fn stops_following_array_bindings_after_unset() {
        let source = "\
#!/bin/bash
cleared_array=(one two)
unset cleared_array
cleared_array=scalar
printf '%s\\n' \"$cleared_array\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn target_rebindings_reset_inherited_array_type() {
        let source = "\
#!/bin/bash
loop_value=(one two)
for loop_value in one two; do
  printf '%s\\n' \"$loop_value\"
done
read_value=(one two)
read read_value <<<input
printf '%s\\n' \"$read_value\"
printf_value=(one two)
printf -v printf_value '%s' input
printf '%s\\n' \"$printf_value\"
local_reset() {
  local local_value=(one two)
  local local_value
  printf '%s\\n' \"$local_value\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_unbound_runtime_arrays_without_bash_prelude() {
        let source = "\
#!/bin/sh
printf '%s\\n' \"$BASH_SOURCE\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$BASH_SOURCE"]
        );
    }

    #[test]
    fn ignores_references_inside_own_array_assignment() {
        let source = "\
#!/bin/bash
TERMUX_PKG_VERSION=(\"$(printf '%s\\n' \"$TERMUX_PKG_VERSION\")\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_references_inside_same_name_array_readers() {
        let source = "\
#!/bin/bash
read -r -a key_value <<<\"$(printf '%s\\n' \"$key_value\")\"
mapfile -t ports_configured < <(printf '%s\\n' \"${ports_configured}\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn imported_bindings_reset_inherited_array_type() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/bash
TERMUX_PKG_VERSION=(\"$(. ./helper.sh; printf '%s\\n' \"$TERMUX_PKG_VERSION\")\")
",
        )
        .unwrap();
        fs::write(&helper, "TERMUX_PKG_VERSION=helper\n").unwrap();

        let source = fs::read_to_string(&main).unwrap();
        let output = Parser::new(&source).parse().unwrap();
        let indexer = Indexer::new(&source, &output);
        let diagnostics = lint_file_at_path(
            &output.file,
            &source,
            &indexer,
            &LinterSettings::for_rule(Rule::QuotedBashSource),
            None,
            Some(&main),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn follows_prior_visible_array_bindings() {
        let source = "\
#!/bin/bash
before_use() {
  printf '%s\\n' \"$future_array\"
}
future_array=(one two)
after_use() {
  printf '%s\\n' \"$future_array\"
}
former_array=(one two)
former_array=scalar
printf '%s\\n' \"$former_array\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$future_array", "$former_array"]
        );
    }

    #[test]
    fn follows_prior_array_bindings_by_source_order() {
        let source = "\
#!/bin/bash
first_function() {
  target=(one two)
}
second_function() {
  local target=$1
  printf '%s\\n' \"$target\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$target"]
        );
    }

    #[test]
    fn reports_runtime_arrays_inside_assign_default_and_error_operands() {
        let source = "\
#!/bin/bash
: ${PROG:=$(basename ${BASH_SOURCE})}
local PATTERN=${2:?$FUNCNAME: a pattern is required}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${BASH_SOURCE}", "$FUNCNAME"]
        );
    }
}
