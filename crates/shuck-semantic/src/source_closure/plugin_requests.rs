//! Zsh plugin request discovery.
//!
//! This module recognizes framework/plugin declarations that imply additional
//! files should be analyzed as part of a zsh source closure. It emits logical
//! `PluginRequest`s only; resolving those requests to concrete files remains the
//! job of the configured `PluginResolver`.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
enum PluginListState {
    Unset,
    Static(Vec<String>),
    Dynamic,
}

#[derive(Debug, Clone)]
struct DetectedPluginBootstrap {
    framework: PluginFramework,
    span: Span,
    root_hint: Option<PathBuf>,
}

pub(super) fn sorted_dependency_paths(paths: &FxHashSet<PathBuf>) -> Vec<PathBuf> {
    let mut sorted = paths.iter().cloned().collect::<Vec<_>>();
    sorted.sort();
    sorted.dedup();
    sorted
}

pub(super) fn collect_plugin_requests(
    model: &SemanticModel,
    file: &File,
    source: &str,
    source_path: &Path,
    plugin_resolver: &(dyn PluginResolver + Send + Sync),
) -> Vec<PluginRequest> {
    if model.shell_profile().dialect != ParseShellDialect::Zsh {
        return Vec::new();
    }

    let mut path_templates = FxHashMap::<Name, SourcePathTemplate>::default();
    let home_dir = env::var_os("HOME").map(PathBuf::from);
    let mut plugin_state = PluginListState::Unset;
    let mut theme_name = None::<String>;
    let mut requests = Vec::new();
    let mut bootstraps = Vec::new();

    for stmt in &file.body.stmts {
        for assignment in top_level_assignments(stmt) {
            if assignment.target.subscript.is_none()
                && let Some(template) = assignment_path_template(
                    assignment,
                    source,
                    &path_templates,
                    home_dir.as_deref(),
                )
            {
                path_templates.insert(assignment.target.name.clone(), template);
            }

            match assignment.target.name.as_str() {
                "plugins" => {
                    plugin_state =
                        plugin_list_state_after_assignment(&plugin_state, assignment, source);
                }
                "ZSH_THEME" => {
                    theme_name = assignment_theme_name(assignment, source);
                }
                _ => {}
            }
        }

        let Some(bootstrap) = detect_plugin_bootstrap(
            stmt,
            source,
            source_path,
            &path_templates,
            home_dir.as_deref(),
        ) else {
            continue;
        };
        bootstraps.push(bootstrap.clone());

        if let PluginListState::Static(names) = &plugin_state {
            for name in names {
                requests.push(PluginRequest {
                    framework: bootstrap.framework.clone(),
                    kind: PluginRequestKind::Plugin,
                    name: name.clone(),
                    span: bootstrap.span,
                    explicit: false,
                    root_hint: bootstrap.root_hint.clone(),
                });
            }
        }

        if let Some(theme_name) = theme_name.as_ref().filter(|name| !name.contains('/')) {
            requests.push(PluginRequest {
                framework: bootstrap.framework.clone(),
                kind: PluginRequestKind::Theme,
                name: theme_name.clone(),
                span: bootstrap.span,
                explicit: false,
                root_hint: bootstrap.root_hint.clone(),
            });
        }
    }

    requests.extend(anchor_configured_plugin_requests(
        plugin_resolver.additional_plugin_requests(source_path),
        &bootstraps,
        file.span,
    ));
    dedup_plugin_requests(requests)
}

fn top_level_assignments(stmt: &shuck_ast::Stmt) -> Vec<&Assignment> {
    match &stmt.command {
        Command::Simple(command) => command.assignments.iter().collect(),
        Command::Decl(command) => command
            .assignments
            .iter()
            .chain(command.operands.iter().filter_map(|operand| match operand {
                DeclOperand::Assignment(assignment) => Some(assignment),
                _ => None,
            }))
            .collect(),
        _ => Vec::new(),
    }
}

fn assignment_path_template(
    assignment: &Assignment,
    source: &str,
    known_templates: &FxHashMap<Name, SourcePathTemplate>,
    home_dir: Option<&Path>,
) -> Option<SourcePathTemplate> {
    let AssignmentValue::Scalar(word) = &assignment.value else {
        return None;
    };
    if let Some(text) = static_word_text(word, source) {
        return static_path_template(&text, home_dir);
    }

    assignment_source_path_template(word, source, false, false, |name, _| {
        resolve_variable_source_template(name, known_templates, home_dir)
    })
}

fn static_path_template(text: &str, home_dir: Option<&Path>) -> Option<SourcePathTemplate> {
    let expanded = expand_static_home_path(text, home_dir)?;
    Some(SourcePathTemplate::Interpolated(vec![
        TemplatePart::Literal(expanded),
    ]))
}

fn resolve_variable_source_template(
    name: &Name,
    known_templates: &FxHashMap<Name, SourcePathTemplate>,
    home_dir: Option<&Path>,
) -> Option<SourcePathTemplate> {
    if name.as_str() == "HOME" {
        return home_dir
            .and_then(|home| static_path_template(&path_to_template_string(home), None));
    }

    known_templates.get(name).cloned()
}

fn expand_static_home_path(text: &str, home_dir: Option<&Path>) -> Option<String> {
    if let Some(home_dir) = home_dir {
        let home = path_to_template_string(home_dir);
        if text == "~" {
            return Some(home);
        }
        if let Some(stripped) = text.strip_prefix("~/") {
            return Some(format!("{home}/{stripped}"));
        }
    }

    Some(text.to_owned())
}

fn plugin_list_state_after_assignment(
    current: &PluginListState,
    assignment: &Assignment,
    source: &str,
) -> PluginListState {
    if assignment.target.subscript.is_some() {
        return PluginListState::Dynamic;
    }
    let AssignmentValue::Compound(array) = &assignment.value else {
        return PluginListState::Dynamic;
    };
    let Some(names) = static_plugin_names(array.elements.as_slice(), source) else {
        return PluginListState::Dynamic;
    };
    if assignment.append {
        let mut combined = match current {
            PluginListState::Unset => Vec::new(),
            PluginListState::Static(existing) => existing.clone(),
            PluginListState::Dynamic => return PluginListState::Dynamic,
        };
        for name in names {
            if !combined.contains(&name) {
                combined.push(name);
            }
        }
        PluginListState::Static(combined)
    } else {
        PluginListState::Static(names)
    }
}

fn static_plugin_names(elements: &[ArrayElem], source: &str) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for element in elements {
        let ArrayElem::Sequential(value) = element else {
            return None;
        };
        let text = static_word_text(value, source)?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        names.push(trimmed.to_owned());
    }
    Some(names)
}

fn assignment_theme_name(assignment: &Assignment, source: &str) -> Option<String> {
    if assignment.append || assignment.target.subscript.is_some() {
        return None;
    }
    let AssignmentValue::Scalar(word) = &assignment.value else {
        return None;
    };
    let text = static_word_text(word, source)?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn detect_plugin_bootstrap(
    stmt: &shuck_ast::Stmt,
    source: &str,
    source_path: &Path,
    known_templates: &FxHashMap<Name, SourcePathTemplate>,
    home_dir: Option<&Path>,
) -> Option<DetectedPluginBootstrap> {
    let Command::Simple(command) = &stmt.command else {
        return None;
    };
    let name = static_word_text(&command.name, source)?;
    if !matches!(name.as_ref(), "source" | ".") {
        return None;
    }
    let arg = command.args.first()?;

    let concrete_path =
        bootstrap_argument_path(arg, source, source_path, known_templates, home_dir);
    if concrete_path
        .as_deref()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "oh-my-zsh.sh")
    {
        return Some(DetectedPluginBootstrap {
            framework: PluginFramework::OhMyZsh,
            span: stmt.span,
            root_hint: concrete_path.and_then(|path| path.parent().map(Path::to_path_buf)),
        });
    }

    if let Some(text) = static_word_text(arg, source) {
        if text.trim().ends_with("oh-my-zsh.sh") {
            return Some(DetectedPluginBootstrap {
                framework: PluginFramework::OhMyZsh,
                span: stmt.span,
                root_hint: framework_root_hint(known_templates, source_path, "ZSH"),
            });
        }
        return None;
    }

    let template = assignment_source_path_template(arg, source, false, false, |name, _| {
        resolve_variable_source_template(name, known_templates, home_dir)
    });
    let looks_like_bootstrap = template
        .as_ref()
        .is_some_and(|template| template_ends_with_literal(template, "oh-my-zsh.sh"));
    looks_like_bootstrap.then(|| DetectedPluginBootstrap {
        framework: PluginFramework::OhMyZsh,
        span: stmt.span,
        root_hint: framework_root_hint(known_templates, source_path, "ZSH"),
    })
}

fn bootstrap_argument_path(
    word: &Word,
    source: &str,
    source_path: &Path,
    known_templates: &FxHashMap<Name, SourcePathTemplate>,
    home_dir: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(text) = static_word_text(word, source) {
        let text = expand_static_home_path(&text, home_dir)?;
        let path = PathBuf::from(text.trim());
        return Some(if path.is_absolute() {
            lexical_normalize_path(&path)
        } else {
            lexical_normalize_path(&source_path.parent()?.join(path))
        });
    }

    let template = source_path_template_with_resolver(word, source, false, false, |name, _| {
        resolve_variable_source_template(name, known_templates, home_dir)
    })?;
    if template.ignored_root {
        return None;
    }
    render_source_path_template(&template.template, source_path)
}

fn render_source_path_template(
    template: &SourcePathTemplate,
    source_path: &Path,
) -> Option<PathBuf> {
    let SourcePathTemplate::Interpolated(parts) = template;
    let mut rendered = String::new();
    for part in parts {
        match part {
            TemplatePart::Literal(text) => rendered.push_str(text),
            TemplatePart::SourceDir => {
                rendered.push_str(&path_to_template_string(source_path.parent()?))
            }
            TemplatePart::SourceFile => rendered.push_str(&path_to_template_string(source_path)),
            TemplatePart::Arg(_) => return None,
        }
    }
    let trimmed = rendered.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    Some(if path.is_absolute() {
        lexical_normalize_path(&path)
    } else {
        lexical_normalize_path(&source_path.parent()?.join(path))
    })
}

fn template_ends_with_literal(template: &SourcePathTemplate, suffix: &str) -> bool {
    match template {
        SourcePathTemplate::Interpolated(parts) => {
            let literal = parts
                .iter()
                .filter_map(|part| match part {
                    TemplatePart::Literal(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>();
            literal.ends_with(suffix)
        }
    }
}

fn framework_root_hint(
    known_templates: &FxHashMap<Name, SourcePathTemplate>,
    source_path: &Path,
    variable_name: &str,
) -> Option<PathBuf> {
    known_templates
        .get(&Name::from(variable_name))
        .and_then(|template| render_source_path_template(template, source_path))
}

fn anchor_configured_plugin_requests(
    requests: Vec<PluginRequest>,
    bootstraps: &[DetectedPluginBootstrap],
    file_span: Span,
) -> Vec<PluginRequest> {
    let file_scope_tail = Span::at(position_after(file_span.end));
    let mut anchored = Vec::new();
    for request in requests {
        if request.kind == PluginRequestKind::Entrypoint {
            anchored.push(PluginRequest {
                // Configured entrypoints are not tied to a source command. Treat
                // them as late file-scope loads so their contract can consume
                // setup assignments made earlier in the file.
                span: file_scope_tail,
                ..request
            });
            continue;
        }

        let matching = bootstraps
            .iter()
            .filter(|bootstrap| bootstrap.framework == request.framework)
            .collect::<Vec<_>>();
        if matching.is_empty() {
            anchored.push(PluginRequest {
                span: file_span,
                ..request
            });
            continue;
        }

        for bootstrap in matching {
            anchored.push(PluginRequest {
                span: bootstrap.span,
                root_hint: bootstrap
                    .root_hint
                    .clone()
                    .or_else(|| request.root_hint.clone()),
                ..request.clone()
            });
        }
    }
    anchored
}

fn position_after(mut position: shuck_ast::Position) -> shuck_ast::Position {
    position.offset += 1;
    position.column += 1;
    position
}

pub(super) fn dedup_plugin_requests(requests: Vec<PluginRequest>) -> Vec<PluginRequest> {
    let mut merged: Vec<PluginRequest> = Vec::new();
    let mut positions = FxHashMap::<
        (
            PluginFramework,
            PluginRequestKind,
            String,
            usize,
            Option<PathBuf>,
        ),
        usize,
    >::default();
    for request in requests {
        let key = (
            request.framework.clone(),
            request.kind,
            request.name.clone(),
            request.span.start.offset,
            request.root_hint.clone(),
        );
        if let Some(&position) = positions.get(&key) {
            if merged[position].explicit {
                continue;
            }
            merged[position] = request;
        } else {
            positions.insert(key, merged.len());
            merged.push(request);
        }
    }
    merged
}
