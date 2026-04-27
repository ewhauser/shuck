use super::*;

impl<'a, 'observer> SemanticModelBuilder<'a, 'observer> {
    pub(super) fn declaration_scope_and_attributes(
        &self,
        builtin: DeclarationBuiltin,
        flags: &FxHashSet<char>,
        global_flag_enabled: bool,
    ) -> (ScopeId, BindingAttributes) {
        let mut attributes = BindingAttributes::empty();
        if matches!(builtin, DeclarationBuiltin::Export) || flags.contains(&'x') {
            attributes |= BindingAttributes::EXPORTED;
        }
        if matches!(builtin, DeclarationBuiltin::Readonly) || flags.contains(&'r') {
            attributes |= BindingAttributes::READONLY;
        }
        if flags.contains(&'i') {
            attributes |= BindingAttributes::INTEGER;
        }
        if flags.contains(&'a') {
            attributes |= BindingAttributes::ARRAY;
        }
        if flags.contains(&'A') {
            attributes |= BindingAttributes::ASSOC;
        }
        if flags.contains(&'n') {
            attributes |= BindingAttributes::NAMEREF;
        }
        if flags.contains(&'l') {
            attributes |= BindingAttributes::LOWERCASE;
        }
        if flags.contains(&'u') {
            attributes |= BindingAttributes::UPPERCASE;
        }

        let global_like = matches!(
            builtin,
            DeclarationBuiltin::Declare | DeclarationBuiltin::Typeset
        ) && global_flag_enabled;
        let local_like = matches!(builtin, DeclarationBuiltin::Local)
            || (matches!(
                builtin,
                DeclarationBuiltin::Declare | DeclarationBuiltin::Typeset
            ) && self.nearest_function_scope().is_some()
                && !global_flag_enabled);

        if local_like {
            attributes |= BindingAttributes::LOCAL;
        }

        (
            if local_like {
                self.nearest_function_scope()
                    .unwrap_or_else(|| self.current_scope())
            } else if global_like {
                self.nearest_execution_scope()
            } else {
                self.current_scope()
            },
            attributes,
        )
    }

    pub(super) fn simple_declaration_scope_and_attributes(
        &self,
        builtin: DeclarationBuiltin,
        flags: &FxHashSet<char>,
        global_flag_enabled: bool,
        flow: FlowState,
    ) -> (ScopeId, BindingAttributes) {
        let (scope, mut attributes) =
            self.declaration_scope_and_attributes(builtin, flags, global_flag_enabled);
        if flow.in_subshell && attributes.contains(BindingAttributes::LOCAL) {
            attributes.remove(BindingAttributes::LOCAL);
            return (self.current_scope(), attributes);
        }

        (scope, attributes)
    }

    pub(super) fn visit_simple_name_only_declaration_operand(
        &mut self,
        builtin: DeclarationBuiltin,
        flags: &FxHashSet<char>,
        global_flag_enabled: bool,
        flow: FlowState,
        name: &Name,
        span: Span,
    ) {
        if flow.in_subshell {
            let (scope, attributes) = self.simple_declaration_scope_and_attributes(
                builtin,
                flags,
                global_flag_enabled,
                flow,
            );
            self.add_binding(
                name,
                BindingKind::Declaration(builtin),
                scope,
                span,
                BindingOrigin::Declaration {
                    definition_span: span,
                },
                attributes,
            );
            return;
        }

        self.visit_name_only_declaration_operand(builtin, flags, global_flag_enabled, name, span);
    }

    pub(super) fn visit_name_only_declaration_operand(
        &mut self,
        builtin: DeclarationBuiltin,
        flags: &FxHashSet<char>,
        global_flag_enabled: bool,
        name: &Name,
        span: Span,
    ) {
        let (scope, attributes) =
            self.declaration_scope_and_attributes(builtin, flags, global_flag_enabled);
        let local_like = attributes.contains(BindingAttributes::LOCAL);
        let existing = self.resolve_reference(name, scope, span.start.offset);

        let reuse_existing = existing.is_some_and(|existing| {
            let existing_binding = &self.bindings[existing.index()];

            !local_like
                || (existing_binding.scope == scope
                    && self.has_uncleared_local_binding_in_scope(name, scope, span.start.offset))
        });

        if reuse_existing {
            let existing = existing.expect("existing binding already checked");
            self.add_reference(name, ReferenceKind::DeclarationName, span);
            self.bindings[existing.index()].attributes |= attributes;
            return;
        }

        let kind = if attributes.contains(BindingAttributes::NAMEREF) {
            BindingKind::Nameref
        } else {
            BindingKind::Declaration(builtin)
        };
        let origin = if matches!(kind, BindingKind::Nameref) {
            BindingOrigin::Nameref {
                definition_span: span,
            }
        } else {
            BindingOrigin::Declaration {
                definition_span: span,
            }
        };
        self.add_binding(name, kind, scope, span, origin, attributes);
    }
}
