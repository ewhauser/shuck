use rustc_hash::FxHashMap;
use shuck_ast::{ArrayElem, Assignment, AssignmentValue, DeclOperand, Span};
use shuck_semantic::ReferenceKind;
use smallvec::SmallVec;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct LocalCrossReference;

impl Violation for LocalCrossReference {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::LocalCrossReference
    }

    fn message(&self) -> String {
        "assignment is reused later in the same declaration".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("split the declaration assignments".to_owned())
    }
}

pub fn local_cross_reference(checker: &mut Checker) {
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let fix = split_declaration_fix(checker.source(), fact);
            declaration_cross_reference_spans(checker, fact)
                .into_iter()
                .map(move |span| (span, fix.clone()))
        })
        .collect::<Vec<_>>();

    for (span, fix) in diagnostics {
        let diagnostic = Diagnostic::new(LocalCrossReference, span);
        if let Some(fix) = fix {
            checker.report_diagnostic_dedup(diagnostic.with_fix(fix));
        } else {
            checker.report_diagnostic_dedup(diagnostic);
        }
    }
}

fn declaration_cross_reference_spans<'a>(
    checker: &Checker<'a>,
    fact: crate::CommandFactRef<'_, 'a>,
) -> Vec<Span> {
    let Some(declaration) = fact.declaration() else {
        return Vec::new();
    };

    let semantic = checker.semantic();
    let mut seen_targets: FxHashMap<&'a str, Span> = FxHashMap::default();
    let mut spans = Vec::new();
    let mut value_spans: SmallVec<[Span; 4]> = SmallVec::new();

    for assignment in declaration.assignment_operands.iter().copied() {
        value_spans.clear();
        push_assignment_value_spans(assignment, &mut value_spans);
        for value_span in &value_spans {
            for reference in semantic.references_in_span(*value_span) {
                if reference.kind == ReferenceKind::DeclarationName {
                    continue;
                }
                if let Some(previous_span) = seen_targets.get(reference.name.as_str()) {
                    spans.push(*previous_span);
                }
            }
        }

        seen_targets.insert(assignment.target.name.as_str(), assignment.target.name_span);
    }

    spans
}

fn push_assignment_value_spans(assignment: &Assignment, spans: &mut SmallVec<[Span; 4]>) {
    match &assignment.value {
        AssignmentValue::Scalar(word) => spans.push(word.span),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                push_array_element_spans(element, spans);
            }
        }
    }
}

fn push_array_element_spans(element: &ArrayElem, spans: &mut SmallVec<[Span; 4]>) {
    match element {
        ArrayElem::Sequential(word) => spans.push(word.span),
        ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
            spans.push(key.span());
            spans.push(value.span);
        }
    }
}

fn split_declaration_fix<'a>(source: &str, fact: crate::CommandFactRef<'_, 'a>) -> Option<Fix> {
    let declaration = fact.declaration()?;
    if !declaration.redirects.is_empty() || declaration.assignment_operands.len() < 2 {
        return None;
    }

    let first_assignment = declaration.assignment_operands.first()?;
    let first_assignment_index = declaration.operands.iter().position(|operand| {
        matches!(operand, DeclOperand::Assignment(assignment) if assignment.span == first_assignment.span)
    })?;

    if !declaration.operands[..first_assignment_index]
        .iter()
        .all(|operand| matches!(operand, DeclOperand::Flag(_)))
        || !declaration.operands[first_assignment_index..]
            .iter()
            .all(|operand| matches!(operand, DeclOperand::Assignment(_)))
    {
        return None;
    }

    let indent = line_indent_before_offset(source, declaration.span.start.offset)?;
    let head = source
        .get(declaration.span.start.offset..first_assignment.span.start.offset)?
        .trim_end();
    if head.is_empty() {
        return None;
    }

    let mut replacement = String::new();
    for assignment in &declaration.assignment_operands {
        if !replacement.is_empty() {
            replacement.push_str(indent);
        }
        replacement.push_str(head);
        replacement.push(' ');
        replacement.push_str(assignment.span.slice(source));
        replacement.push('\n');
    }

    Some(Fix::unsafe_edit(Edit::replacement(
        replacement,
        declaration.span,
    )))
}

fn line_indent_before_offset(source: &str, offset: usize) -> Option<&str> {
    let line_start = source[..offset]
        .rfind('\n')
        .map_or(0, |newline| newline + '\n'.len_utf8());
    let indent = source.get(line_start..offset)?;
    indent
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t'))
        .then_some(indent)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn anchors_on_prior_assignment_names_in_declarations() {
        let source = "\
#!/bin/sh
local a=1 b=$a c=$b
declare x=1 y=$(printf '%s' \"$x\")
readonly p=1 q=(\"$p\")
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::LocalCrossReference));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["a", "b", "x", "p"]
        );
    }

    #[test]
    fn prefers_the_most_recent_prior_assignment_for_reused_names() {
        let source = "\
#!/bin/sh
local a=1 a=2 c=$a
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::LocalCrossReference));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.start.offset,
            source.find("a=2").unwrap()
        );
    }

    #[test]
    fn ignores_associative_array_keys_inside_arithmetic_subscripts() {
        let source = "\
#!/bin/bash
f() {
  declare -A box=([m_width]=1 [mem_col]=5)
  local m_width=1 mem_line=$((box[mem_col]+box[m_width]))
}
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::LocalCrossReference));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_associative_array_keys_after_arithmetic_writes() {
        let source = "\
#!/bin/bash
f() {
  declare -A box=([key]=1)
  (( box[seed] = 1 ))
  local key=1 value=$((box[key]))
}
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::LocalCrossReference));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_by_splitting_declaration_assignments() {
        let source = "\
#!/bin/sh
f() {
  local -r a=1 b=$a c=$b
  declare x=1 y=$(printf '%s' \"$x\")
}
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LocalCrossReference),
            Applicability::Unsafe,
        );

        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
f() {
  local -r a=1
  local -r b=$a
  local -r c=$b
  declare x=1
  declare y=$(printf '%s' \"$x\")
}
"
        );
        assert_eq!(result.fixes_applied, 2);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_declarations_unchanged() {
        let source = "\
#!/bin/sh
local a=1 b=$a
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LocalCrossReference),
            Applicability::Safe,
        );

        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixes_applied, 0);
    }

    #[test]
    fn skips_fix_for_mixed_or_inline_declarations() {
        let source = "\
#!/bin/sh
if local a=1 b=$a; then
  :
fi
local keep a=1 b=$a
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LocalCrossReference),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.diagnostics.len(), 2);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C136.sh").as_path(),
            &LinterSettings::for_rule(Rule::LocalCrossReference),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C136_fix_C136.sh", result);
        Ok(())
    }
}
