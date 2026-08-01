use super::*;

impl<'a, 'idx, 'observer> SemanticModelBuilder<'a, 'idx, 'observer> {
    pub(super) fn add_reference(
        &mut self,
        name: &Name,
        kind: ReferenceKind,
        span: Span,
    ) -> ReferenceId {
        self.add_reference_with_name_span(name, kind, span, Span::new())
    }

    pub(super) fn add_reference_with_name_span(
        &mut self,
        name: &Name,
        kind: ReferenceKind,
        span: Span,
        name_span: Span,
    ) -> ReferenceId {
        let span = self.normalize_reference_span(name, kind, span);
        let name_span = if name_span.to_range().is_empty() {
            self.reference_name_span(name, span).unwrap_or(span)
        } else {
            name_span
        };
        let id = ReferenceId(self.references.len() as u32);
        let scope = self.current_scope();
        let resolved = self.resolve_reference(name, scope, span.start.offset());
        let predefined_runtime = resolved.is_none() && self.runtime.is_preinitialized(name);

        self.references.push(Reference {
            id,
            name: name.clone(),
            kind,
            scope,
            span,
            name_span,
        });
        self.reference_index
            .entry(name.clone())
            .or_default()
            .push(id);
        if self.guarded_parameter_operand_depth > 0 {
            self.guarded_parameter_refs.insert(id);
        }
        if self.defaulting_parameter_operand_depth > 0 {
            self.defaulting_parameter_operand_refs.insert(id);
        }
        if let Some(command) = self.command_stack.last().copied() {
            self.command_references
                .entry(SpanKey::new(command))
                .or_default()
                .push(id);
        }

        if let Some(binding) = resolved {
            self.resolved.insert(id, binding);
            self.bindings[binding.index()].references.push(id);
        } else if predefined_runtime {
            self.predefined_runtime_refs.insert(id);
        } else {
            self.unresolved.push(id);
        }

        let reference = &self.references[id.index()];
        let resolved_binding = resolved.map(|binding| &self.bindings[binding.index()]);
        self.observer.record_reference(reference, resolved_binding);
        id
    }

    pub(super) fn normalize_reference_span(
        &self,
        name: &Name,
        kind: ReferenceKind,
        span: Span,
    ) -> Span {
        if span.end.offset() >= self.source.len() {
            return span;
        }

        let syntax = span.slice(self.source);
        if matches!(kind, ReferenceKind::Expansion)
            && unbraced_parameter_reference_matches(syntax, name.as_str())
        {
            return span;
        }
        if !reference_kind_uses_braced_parameter_syntax(kind) {
            return span;
        }
        if let Some(start_rel) = syntax.find('$') {
            let candidate = &syntax[start_rel..];
            if unbraced_parameter_reference_matches(candidate, name.as_str()) {
                let start_offset = span.start.offset() + start_rel;
                let end_offset = start_offset + '$'.len_utf8() + name.as_str().len();
                if let Some(start) = shifted_position_within_line(span.start, syntax, start_rel)
                    && let Some(matched) = self.source.get(start_offset..end_offset)
                    && let Some(end) = shifted_position_within_line(start, matched, matched.len())
                    && start.offset() < end.offset()
                {
                    return Span::from_positions(start, end);
                }
                if let Some((start, end)) =
                    self.source_positions_for_offsets(start_offset, end_offset)
                    && start.offset() < end.offset()
                {
                    return Span::from_positions(start, end);
                }
            }
        }
        let Some(start_rel) = syntax.find("${") else {
            return self
                .recover_unbraced_reference_span(name, span)
                .or_else(|| self.recover_braced_reference_span(name, span))
                .unwrap_or(span);
        };
        if self.source.as_bytes().get(span.end.offset()) != Some(&b'}') {
            return self
                .recover_braced_reference_span(name, span)
                .unwrap_or(span);
        }

        let start_offset = span.start.offset() + start_rel;
        let end_offset = span.end.offset() + '}'.len_utf8();
        if let Some(start) = shifted_position_within_line(span.start, syntax, start_rel) {
            // The closing brace is a single-column character after the span.
            let end = Position::at(span.end.line(), span.end.column() + 1, end_offset);
            if start.offset() < end.offset() {
                return Span::from_positions(start, end);
            }
            return span;
        }
        let Some((start, end)) = self.source_positions_for_offsets(start_offset, end_offset) else {
            return span;
        };
        if start.offset() < end.offset() {
            Span::from_positions(start, end)
        } else {
            span
        }
    }

    fn reference_name_span(&self, name: &Name, span: Span) -> Option<Span> {
        let syntax = span.slice(self.source);
        if syntax == name.as_str() {
            return (span.start.offset() < span.end.offset()).then_some(span);
        }
        // `$name` and `${name}` spans dominate; match them directly instead of
        // running a substring search per reference.
        let (relative_start, name_len) =
            if syntax.as_bytes().first() == Some(&b'$') && syntax[1..] == *name.as_str() {
                (1, name.as_str().len())
            } else if let Some(rest) = syntax.strip_prefix("${")
                && rest.strip_suffix('}') == Some(name.as_str())
            {
                (2, name.as_str().len())
            } else if let Some(start) = syntax.find(name.as_str()) {
                (start, name.as_str().len())
            } else if let Some(stripped) = name.as_str().strip_prefix('+') {
                (syntax.find(stripped)?, stripped.len())
            } else {
                return None;
            };
        let start_offset = span.start.offset() + relative_start;
        let end_offset = start_offset + name_len;
        if let Some(start) = shifted_position_within_line(span.start, syntax, relative_start) {
            let matched = &syntax[relative_start..relative_start + name_len];
            if let Some(end) = shifted_position_within_line(start, matched, name_len) {
                return (start.offset() < end.offset()).then(|| Span::from_positions(start, end));
            }
        }
        let (start, end) = self.source_positions_for_offsets(start_offset, end_offset)?;
        (start.offset() < end.offset()).then(|| Span::from_positions(start, end))
    }

    pub(super) fn recover_braced_reference_span(&self, name: &Name, span: Span) -> Option<Span> {
        if name.is_empty() || span.start.offset() >= self.source.len() {
            return None;
        }

        let name = name.as_str();
        let search_end = self
            .source
            .get(span.start.offset()..)?
            .find('\n')
            .map(|relative| span.start.offset() + relative)
            .unwrap_or(self.source.len());

        let search = self.source.get(span.start.offset()..search_end)?;
        let mut needle = compact_str::CompactString::const_new("${");
        needle.push_str(name);
        for (start_rel, _) in search.match_indices(needle.as_str()) {
            let start_offset = span.start.offset() + start_rel;
            if braced_parameter_start_matches(self.source, start_offset, name)
                && let Some(end_offset) =
                    braced_parameter_end_offset(self.source, start_offset, search_end)
                && let Some((start, end)) =
                    self.source_positions_for_offsets(start_offset, end_offset)
                && start.offset() < end.offset()
            {
                return Some(Span::from_positions(start, end));
            }
        }

        self.recover_braced_reference_span_on_line(name, &needle, span)
    }

    pub(super) fn recover_unbraced_reference_span(&self, name: &Name, span: Span) -> Option<Span> {
        if name.is_empty() || span.start.offset() >= self.source.len() {
            return None;
        }

        let (line_start_offset, line) =
            source_line(self.source, self.line_index, span.start.line())?;
        let name = name.as_str();
        let mut best = None::<(usize, usize, usize)>;
        for (start, _) in line.match_indices('$') {
            if !unbraced_parameter_start_matches(line, start, name) {
                continue;
            }
            let end = start + '$'.len_utf8() + name.len();
            let column = line.get(..start)?.chars().count() + 1;
            let distance = column.abs_diff(span.start.column());
            if best
                .as_ref()
                .is_none_or(|(_, _, best_distance)| distance < *best_distance)
            {
                best = Some((start, end, distance));
            }
        }

        let (start, end, _) = best?;
        let start_offset = line_start_offset + start;
        let end_offset = line_start_offset + end;
        let (start, end) = self.source_positions_for_offsets(start_offset, end_offset)?;
        (start.offset() < end.offset()).then(|| Span::from_positions(start, end))
    }

    pub(super) fn recover_braced_reference_span_on_line(
        &self,
        name: &str,
        needle: &str,
        span: Span,
    ) -> Option<Span> {
        let (line_start_offset, line) =
            source_line(self.source, self.line_index, span.start.line())?;
        let mut best = None::<(usize, usize, usize)>;
        for (start, _) in line.match_indices(needle) {
            if !braced_parameter_start_matches(line, start, name) {
                continue;
            }
            let Some(end) = braced_parameter_end_offset(line, start, line.len()) else {
                continue;
            };
            let column = line.get(..start)?.chars().count() + 1;
            let distance = column.abs_diff(span.start.column());
            if best
                .as_ref()
                .is_none_or(|(_, _, best_distance)| distance < *best_distance)
            {
                best = Some((start, end, distance));
            }
        }

        let (start, end, _) = best?;
        let start_offset = line_start_offset + start;
        let end_offset = line_start_offset + end;
        let (start, end) = self.source_positions_for_offsets(start_offset, end_offset)?;
        (start.offset() < end.offset()).then(|| Span::from_positions(start, end))
    }

    pub(super) fn source_positions_for_offsets(
        &self,
        start: usize,
        end: usize,
    ) -> Option<(Position, Position)> {
        if start > end || end > self.source.len() {
            return None;
        }
        Some((
            self.source_position_at_offset(start)?,
            self.source_position_at_offset(end)?,
        ))
    }

    pub(super) fn source_position_at_offset(&self, offset: usize) -> Option<Position> {
        source_position_at_offset(self.source, self.line_index, offset)
    }

    pub(super) fn add_parameter_default_binding(&mut self, reference: &VarRef) {
        let mut attributes = binding_attributes_for_var_ref(reference);
        if reference.subscript.is_some()
            && !attributes.contains(BindingAttributes::ASSOC)
            && self
                .resolve_reference(
                    &reference.name,
                    self.current_scope(),
                    reference.name_span.start.offset(),
                )
                .map(|binding_id| {
                    let binding = &self.bindings[binding_id.index()];
                    binding.attributes.contains(BindingAttributes::ASSOC)
                        && !self.binding_was_cleared_before_lookup(
                            binding,
                            self.current_scope(),
                            reference.name_span.start.offset(),
                        )
                })
                .unwrap_or(false)
        {
            attributes |= BindingAttributes::ARRAY | BindingAttributes::ASSOC;
        }

        self.add_binding(
            &reference.name,
            BindingKind::ParameterDefaultAssignment,
            self.current_scope(),
            reference.span,
            BindingOrigin::ParameterDefaultAssignment {
                definition_span: reference.span,
                target_span: reference.name_span,
            },
            attributes,
        );
    }

    pub(super) fn add_reference_if_bound(&mut self, name: &Name, kind: ReferenceKind, span: Span) {
        if self
            .resolve_reference(name, self.current_scope(), span.start.offset())
            .is_some()
        {
            self.add_reference(name, kind, span);
        }
    }

    pub(super) fn newly_added_reference_ids_reading_name(
        &self,
        name: &Name,
        start: usize,
    ) -> Vec<ReferenceId> {
        self.references[start..]
            .iter()
            .filter(|reference| reference.name == *name)
            .map(|reference| reference.id)
            .collect()
    }

    pub(super) fn resolve_reference(
        &self,
        name: &Name,
        scope: ScopeId,
        offset: usize,
    ) -> Option<BindingId> {
        for scope in ancestor_scopes(&self.scopes, scope) {
            let Some(bindings) = self.scopes[scope.index()].bindings.get(name) else {
                continue;
            };

            if self.completed_scopes.contains(&scope) {
                if let Some(binding) = bindings.last().copied() {
                    return Some(binding);
                }
            } else {
                for binding in bindings.iter().rev().copied() {
                    if self.bindings[binding.index()].span.start.offset() <= offset {
                        return Some(binding);
                    }
                }
            }
        }
        None
    }

    pub(super) fn compute_heuristic_unused_assignments(&self) -> Vec<BindingId> {
        self.bindings
            .iter()
            .filter(|binding| {
                !matches!(
                    binding.kind,
                    BindingKind::FunctionDefinition | BindingKind::Imported
                ) && binding.references.is_empty()
                    && !binding
                        .attributes
                        .contains(BindingAttributes::SELF_REFERENTIAL_READ)
                    && !binding
                        .attributes
                        .contains(BindingAttributes::EXTERNALLY_CONSUMED)
            })
            .map(|binding| binding.id)
            .collect()
    }
}

/// Shift `base` forward by `byte_offset` bytes of `text` (starting at
/// `base`), staying on the same line. Returns `None` when the prefix spans a
/// line break so callers can fall back to a full line-index lookup.
fn shifted_position_within_line(
    base: Position,
    text: &str,
    byte_offset: usize,
) -> Option<Position> {
    let prefix = text.get(..byte_offset)?;
    if prefix.as_bytes().contains(&b'\n') {
        return None;
    }
    let chars = if prefix.is_ascii() {
        prefix.len()
    } else {
        prefix.chars().count()
    };
    Some(Position::at(
        base.line(),
        base.column() + chars,
        base.offset() + byte_offset,
    ))
}

fn source_position_at_offset(
    source: &str,
    line_index: &LineIndex,
    offset: usize,
) -> Option<Position> {
    if offset > source.len() || !source.is_char_boundary(offset) {
        return None;
    }

    let line = line_index.line_number(TextSize::new(offset as u32));
    let line_start = usize::from(line_index.line_start(line)?);
    let prefix = source.get(line_start..offset)?;
    let column = if prefix.is_ascii() {
        prefix.len()
    } else {
        prefix.chars().count()
    } + 1;
    Some(Position::at(line, column, offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_position_lookup_uses_precomputed_line_starts() {
        let source = "alpha\nb\u{e9}ta\n";
        let line_index = LineIndex::new(source);

        assert_eq!(
            source_position_at_offset(source, &line_index, 0),
            Some(Position::at(1, 1, 0))
        );
        let beta_offset = source.find('b').expect("expected second line");
        assert_eq!(
            source_position_at_offset(source, &line_index, beta_offset),
            Some(Position::at(2, 1, beta_offset))
        );
        let after_e_acute = beta_offset + "b\u{e9}".len();
        assert_eq!(
            source_position_at_offset(source, &line_index, after_e_acute),
            Some(Position::at(2, 3, after_e_acute))
        );
        assert_eq!(
            source_position_at_offset(source, &line_index, source.len()),
            Some(Position::at(3, 1, source.len()))
        );
    }
}
