use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Name, Span};
use shuck_semantic::{
    Binding, BindingAttributes, BindingId, BindingKind, BindingOrigin, DeclarationBuiltin, ScopeId,
    ScopeKind,
};

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct MutableGlobal {
    pub name: CompactString,
}

impl Violation for MutableGlobal {
    fn rule() -> Rule {
        Rule::MutableGlobal
    }

    fn message(&self) -> String {
        format!(
            "global variable `{}` is written more than once; make it readonly or keep later writes local",
            self.name
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlobalWriteKind {
    File,
    Function,
}

#[derive(Debug, Clone, Copy)]
struct GlobalWrite {
    binding_id: BindingId,
    kind: GlobalWriteKind,
}

pub fn mutable_global(checker: &mut Checker) {
    if checker.shell() != ShellDialect::Bash {
        return;
    }

    let semantic = checker.semantic();
    let mut writes_by_name = FxHashMap::<Name, Vec<GlobalWrite>>::default();

    for binding in semantic.bindings() {
        if !is_mutable_global_write_candidate(binding) {
            continue;
        }

        let Some(kind) = global_write_kind(checker, binding) else {
            continue;
        };

        writes_by_name
            .entry(binding.name.clone())
            .or_default()
            .push(GlobalWrite {
                binding_id: binding.id,
                kind,
            });
    }

    let allow_conditional_init = checker.rule_options().c159.allow_conditional_init;
    let mut reported = FxHashSet::<BindingId>::default();
    let mut reportable_bindings = Vec::new();

    for writes in writes_by_name.values_mut() {
        writes.sort_unstable_by_key(|write| semantic.binding(write.binding_id).span.start.offset);

        let Some(first_file_write) = writes
            .iter()
            .find(|write| write.kind == GlobalWriteKind::File)
            .map(|write| write.binding_id)
        else {
            continue;
        };

        for write in writes.iter().copied() {
            let binding = semantic.binding(write.binding_id);

            if write.kind == GlobalWriteKind::File && write.binding_id == first_file_write {
                continue;
            }
            if allow_conditional_init && is_conditional_default_init(binding, checker.source()) {
                continue;
            }
            if reported.insert(write.binding_id) {
                reportable_bindings.push(write.binding_id);
            }
        }
    }

    reportable_bindings
        .sort_unstable_by_key(|binding_id| semantic.binding(*binding_id).span.start.offset);

    for binding_id in reportable_bindings {
        let binding = semantic.binding(binding_id);
        checker.report(
            MutableGlobal {
                name: binding.name.as_str().into(),
            },
            report_span_for_binding(binding),
        );
    }
}

fn global_write_kind(checker: &Checker<'_>, binding: &Binding) -> Option<GlobalWriteKind> {
    let semantic = checker.semantic();

    if matches!(semantic.scope_kind(binding.scope), ScopeKind::File) {
        return Some(GlobalWriteKind::File);
    }

    if checker
        .semantic_analysis()
        .scope_runs_in_transient_context(binding.scope)
    {
        return None;
    }

    let function_scope =
        semantic.enclosing_function_scope_without_transient_boundary(binding.scope)?;
    if binding.attributes.contains(BindingAttributes::LOCAL)
        || has_prior_function_declaration(checker, binding, function_scope)
    {
        return None;
    }

    Some(GlobalWriteKind::Function)
}

fn has_prior_function_declaration(
    checker: &Checker<'_>,
    binding: &Binding,
    function_scope: ScopeId,
) -> bool {
    let semantic = checker.semantic();
    semantic
        .bindings_for(&binding.name)
        .iter()
        .copied()
        .any(|id| {
            let candidate = semantic.binding(id);
            candidate.scope == function_scope
                && candidate.span.start.offset < binding.span.start.offset
                && is_function_local_declaration(candidate)
                && declaration_runs_before_binding(checker, candidate, binding, function_scope)
        })
}

fn declaration_runs_before_binding(
    checker: &Checker<'_>,
    declaration: &Binding,
    binding: &Binding,
    function_scope: ScopeId,
) -> bool {
    let analysis = checker.semantic_analysis();
    let declaration_blocks = analysis.reachable_blocks_for_binding(declaration.id);
    let binding_blocks = analysis.reachable_blocks_for_binding(binding.id);
    if declaration_blocks.is_empty() {
        return false;
    }
    if binding_blocks.is_empty() {
        return true;
    }

    let declaration_blocks = declaration_blocks.into_iter().collect::<FxHashSet<_>>();
    let uncovered_binding_blocks = binding_blocks
        .into_iter()
        .filter(|block| !declaration_blocks.contains(block))
        .collect::<Vec<_>>();
    if uncovered_binding_blocks.is_empty() {
        return true;
    }

    let Some(entry_block) = analysis.cfg().scope_entry(function_scope) else {
        return true;
    };

    !analysis.blocks_have_path_avoiding(
        &[entry_block],
        &uncovered_binding_blocks,
        &declaration_blocks,
    )
}

fn is_function_local_declaration(binding: &Binding) -> bool {
    binding.attributes.contains(BindingAttributes::LOCAL)
        || matches!(
            binding.kind,
            BindingKind::Declaration(
                DeclarationBuiltin::Declare
                    | DeclarationBuiltin::Local
                    | DeclarationBuiltin::Readonly
                    | DeclarationBuiltin::Typeset
            ) | BindingKind::Nameref
        )
}

fn is_mutable_global_write_candidate(binding: &Binding) -> bool {
    if binding
        .attributes
        .intersects(BindingAttributes::READONLY | BindingAttributes::LOCAL)
    {
        return false;
    }

    match binding.kind {
        BindingKind::Assignment
        | BindingKind::AppendAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ZparseoptsTarget
        | BindingKind::ArithmeticAssignment
        | BindingKind::ParameterDefaultAssignment => true,
        BindingKind::Declaration(_) => binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED),
        BindingKind::FunctionDefinition | BindingKind::Imported | BindingKind::Nameref => false,
    }
}

fn is_conditional_default_init(binding: &Binding, source: &str) -> bool {
    if matches!(binding.kind, BindingKind::ParameterDefaultAssignment) {
        return true;
    }

    binding
        .attributes
        .contains(BindingAttributes::SELF_REFERENTIAL_READ)
        && matches!(binding.origin, BindingOrigin::Assignment { .. })
        && assignment_value_uses_self_default_operator(binding, source)
}

fn assignment_value_uses_self_default_operator(binding: &Binding, source: &str) -> bool {
    let Some(value) = source
        .get(binding.span.end.offset..)
        .and_then(|remainder| remainder.strip_prefix('='))
    else {
        return false;
    };
    let value_word = assignment_value_word(value);
    let value_word = assignment_value_without_trailing_comment(value_word);
    value_uses_only_self_default_parameter_expansions(value_word, binding.name.as_str())
}

fn assignment_value_word(value: &str) -> &str {
    let bytes = value.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' | b'\r' | b';' => return &value[..index],
            b'\'' => index = skip_single_quoted_assignment_value(bytes, index + 1),
            b'"' => index = skip_double_quoted_assignment_value(bytes, index + 1),
            b'`' => index = skip_backtick_assignment_value(bytes, index + 1),
            b'\\' => index = (index + 2).min(bytes.len()),
            b'$' => index = skip_dollar_assignment_expansion(bytes, index),
            _ => index += 1,
        }
    }

    value
}

fn assignment_value_without_trailing_comment(value: &str) -> &str {
    let mut escaped = false;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut comment_can_start = false;

    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            comment_can_start = false;
            continue;
        }

        if in_single_quote {
            if ch == '\'' {
                in_single_quote = false;
            }
            continue;
        }

        if in_double_quote {
            match ch {
                '\\' => escaped = true,
                '"' => in_double_quote = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '#' if comment_can_start => return &value[..index],
            '\\' => {
                escaped = true;
                comment_can_start = false;
            }
            '\'' => {
                in_single_quote = true;
                comment_can_start = false;
            }
            '"' => {
                in_double_quote = true;
                comment_can_start = false;
            }
            ' ' | '\t' => comment_can_start = true,
            _ => comment_can_start = false,
        }
    }

    value
}

fn skip_single_quoted_assignment_value(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

fn skip_double_quoted_assignment_value(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = (index + 2).min(bytes.len()),
            b'"' => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn skip_backtick_assignment_value(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = (index + 2).min(bytes.len()),
            b'`' => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn skip_dollar_assignment_expansion(bytes: &[u8], index: usize) -> usize {
    let Some(next) = bytes.get(index + 1).copied() else {
        return bytes.len();
    };

    match next {
        b'{' => skip_balanced_assignment_value(bytes, index + 2, b'{', b'}'),
        b'(' => skip_balanced_assignment_value(bytes, index + 2, b'(', b')'),
        b'[' => skip_balanced_assignment_value(bytes, index + 2, b'[', b']'),
        _ => index + 1,
    }
}

fn skip_balanced_assignment_value(bytes: &[u8], mut index: usize, open: u8, close: u8) -> usize {
    let mut depth = 1usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' => index = skip_single_quoted_assignment_value(bytes, index + 1),
            b'"' => index = skip_double_quoted_assignment_value(bytes, index + 1),
            b'`' => index = skip_backtick_assignment_value(bytes, index + 1),
            b'\\' => index = (index + 2).min(bytes.len()),
            b'$' => index = skip_dollar_assignment_expansion(bytes, index),
            byte if byte == open => {
                depth += 1;
                index += 1;
            }
            byte if byte == close => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    return index;
                }
            }
            _ => index += 1,
        }
    }
    bytes.len()
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct SelfParameterExpansionUses {
    default_operator: bool,
    other_read: bool,
}

fn value_uses_only_self_default_parameter_expansions(value: &str, name: &str) -> bool {
    let uses = self_parameter_expansion_uses(value, name);
    uses.default_operator && !uses.other_read
}

fn self_parameter_expansion_uses(value: &str, name: &str) -> SelfParameterExpansionUses {
    let bytes = value.as_bytes();
    let name_bytes = name.as_bytes();
    let mut uses = SelfParameterExpansionUses::default();
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'\'' => index = skip_single_quoted_assignment_value(bytes, index + 1),
            b'\\' => index = (index + 2).min(bytes.len()),
            b'$' => {
                if bytes.get(index + 1) == Some(&b'{') {
                    let name_start = index + 2;
                    let name_end = name_start + name_bytes.len();
                    if bytes
                        .get(name_start..name_end)
                        .is_some_and(|candidate| candidate == name_bytes)
                        && braced_parameter_name_has_boundary(bytes, name_end)
                    {
                        if braced_parameter_tail_starts_default_operator(&bytes[name_end..]) {
                            uses.default_operator = true;
                        } else {
                            uses.other_read = true;
                        }
                    }
                } else {
                    let name_start = index + 1;
                    let name_end = name_start + name_bytes.len();
                    if bytes
                        .get(name_start..name_end)
                        .is_some_and(|candidate| candidate == name_bytes)
                        && bare_parameter_name_has_boundary(bytes, name_end)
                    {
                        uses.other_read = true;
                    }
                }
                index += 1;
            }
            _ => index += 1,
        }
    }

    uses
}

fn braced_parameter_name_has_boundary(bytes: &[u8], index: usize) -> bool {
    !bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

fn bare_parameter_name_has_boundary(bytes: &[u8], index: usize) -> bool {
    !bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

fn braced_parameter_tail_starts_default_operator(tail: &[u8]) -> bool {
    matches!(
        tail,
        [b':', b'-', ..] | [b':', b'=', ..] | [b'-', ..] | [b'=', ..]
    )
}

fn report_span_for_binding(binding: &Binding) -> Span {
    match binding.origin {
        BindingOrigin::LoopVariable {
            definition_span, ..
        }
        | BindingOrigin::Assignment {
            definition_span, ..
        }
        | BindingOrigin::ParameterDefaultAssignment { definition_span }
        | BindingOrigin::Imported { definition_span }
        | BindingOrigin::FunctionDefinition { definition_span }
        | BindingOrigin::BuiltinTarget {
            definition_span, ..
        }
        | BindingOrigin::Declaration { definition_span }
        | BindingOrigin::Nameref { definition_span } => definition_span,
        BindingOrigin::ArithmeticAssignment { target_span, .. } => target_span,
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_repeated_top_level_assignments_and_function_reassignments() {
        let source = "\
#!/bin/bash
count=0
count=1
refresh() {
  count=2
}
refresh
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MutableGlobal));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["count", "count"]
        );
    }

    #[test]
    fn ignores_single_top_level_assignments() {
        let source = "#!/bin/bash\ncount=0\necho \"$count\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MutableGlobal));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_readonly_globals() {
        let source = "\
#!/bin/bash
readonly COUNT=0
declare -r NAME=demo
show() {
  echo \"$COUNT\" \"$NAME\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MutableGlobal));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_function_local_shadowing() {
        let source = "\
#!/bin/bash
value=0
update() {
  local value
  value=1
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MutableGlobal));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_function_assignment_when_prior_local_declaration_is_not_guaranteed() {
        let source = "\
#!/bin/bash
value=0
update() {
  if [[ $make_local ]]; then
    local value
  fi
  value=1
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MutableGlobal));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "value");
        assert_eq!(diagnostics[0].span.start.line, 7);
    }

    #[test]
    fn reports_function_assignment_when_prior_local_declaration_is_unreachable() {
        let source = "\
#!/bin/bash
value=0
update() {
  if [[ $stop ]]; then
    return
    local value
  fi
  value=1
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MutableGlobal));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "value");
        assert_eq!(diagnostics[0].span.start.line, 8);
    }

    #[test]
    fn ignores_branch_local_assignment_after_branch_local_declaration() {
        let source = "\
#!/bin/bash
value=0
update() {
  if [[ $make_local ]]; then
    local value
    value=1
  fi
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MutableGlobal));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn allows_conditional_default_initializers_by_default() {
        let source = "\
#!/bin/bash
mode=dev
mode=${mode:-prod}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MutableGlobal));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_self_referential_parameter_rewrites() {
        let source = "\
#!/bin/bash
mode=dev
mode=${mode#dev}
path=${path:-/tmp}
path=${path%/}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MutableGlobal));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["mode", "path"]
        );
    }

    #[test]
    fn reports_mixed_self_default_and_rewrite_assignments() {
        let source = "\
#!/bin/bash
mode=dev
mode=${mode#d}${mode:-prod}
path=old
path=${path:-/tmp}${other:-$path}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MutableGlobal));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["mode", "path"]
        );
    }

    #[test]
    fn ignores_default_operators_in_trailing_comments() {
        let source = "\
#!/bin/bash
mode=dev
mode=${mode#dev} # ${mode:-prod}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MutableGlobal));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "mode");
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn allows_conditional_default_initializers_after_quoted_or_substitution_semicolons() {
        let source = "\
#!/bin/bash
mode=dev
mode=\"x;${mode:-prod}\"
path=old
path=\"$(printf '%s;' value)${path:-/tmp}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MutableGlobal));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn still_ignores_default_operators_after_real_assignment_terminators() {
        let source = "\
#!/bin/bash
mode=dev
mode=${mode#dev}; echo ${mode:-prod}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MutableGlobal));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "mode");
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn option_can_report_conditional_default_initializers() {
        let source = "\
#!/bin/bash
mode=dev
mode=${mode:-prod}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MutableGlobal).with_c159_allow_conditional_init(false),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "mode");
    }

    #[test]
    fn later_assignment_after_conditional_baseline_is_reported() {
        let source = "\
#!/bin/bash
mode=${mode:-dev}
mode=prod
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MutableGlobal));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "mode");
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn ignores_other_shells() {
        let source = "\
#!/bin/sh
count=0
count=1
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MutableGlobal).with_shell(ShellDialect::Sh),
        );

        assert!(diagnostics.is_empty());
    }
}
