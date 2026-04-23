use shuck_ast::Span;
use shuck_semantic::{Binding, BindingAttributes, BindingKind, DeclarationBuiltin, Reference};

use crate::{Checker, Rule, Violation};

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
    let spans = checker
        .facts()
        .plain_unindexed_reference_spans()
        .iter()
        .copied()
        .filter(|span| plain_reference_is_array_like(semantic, *span))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || QuotedBashSource);
}

fn plain_reference_is_array_like(semantic: &shuck_semantic::SemanticModel, span: Span) -> bool {
    semantic
        .references()
        .iter()
        .any(|reference| reference.span == span && reference_is_array_like(semantic, reference))
}

fn reference_is_array_like(
    semantic: &shuck_semantic::SemanticModel,
    reference: &Reference,
) -> bool {
    semantic.reference_is_predefined_runtime_array(reference.id)
        || is_bash_runtime_array_name(reference.name.as_str())
        || semantic
            .resolved_binding(reference.id)
            .is_some_and(|binding| {
                semantic.binding_visible_at(binding.id, reference.span)
                    && ((binding_is_array_like(binding)
                        && !binding_reads_its_own_command_input(semantic, binding, reference))
                        || binding_inherits_indexed_array_type(semantic, binding))
            })
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
    semantic
        .bindings_for(&binding.name)
        .iter()
        .copied()
        .filter(|candidate_id| {
            let candidate = semantic.binding(*candidate_id);
            candidate.span.start.offset < binding.span.start.offset
                && semantic.binding_visible_at(*candidate_id, binding.span)
        })
        .map(|candidate_id| semantic.binding(candidate_id))
        .next_back()
        .is_some_and(binding_is_sticky_indexed_array)
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

fn binding_reads_its_own_command_input(
    semantic: &shuck_semantic::SemanticModel,
    binding: &Binding,
    reference: &Reference,
) -> bool {
    matches!(
        binding.kind,
        BindingKind::ReadTarget | BindingKind::MapfileTarget
    ) && binding.span.start.offset < reference.span.start.offset
        && semantic.binding_and_reference_share_command(binding.id, reference.id)
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
    use crate::{LinterSettings, Rule};

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
arr=(one two)
x=$BASH_SOURCE
y=${BASH_SOURCE}
z=\"${BASH_SOURCE[0]}\"
q=\"${BASH_SOURCE[@]}\"
r=\"${BASH_SOURCE[*]}\"
s=\"${BASH_SOURCE%/*}\"
t=\"${BASH_SOURCE:-fallback}\"
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
}
