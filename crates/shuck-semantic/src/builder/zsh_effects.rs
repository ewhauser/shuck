use super::*;

pub(super) fn recorded_command_info(
    command: &Command,
    source: &str,
    bash_runtime_vars_enabled: bool,
) -> RecordedCommandInfo {
    match command {
        Command::Simple(command) => {
            recorded_simple_command_info(command, source, bash_runtime_vars_enabled)
        }
        Command::Builtin(_)
        | Command::Decl(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => RecordedCommandInfo::default(),
    }
}

pub(super) fn recorded_simple_command_info(
    command: &shuck_ast::SimpleCommand,
    source: &str,
    bash_runtime_vars_enabled: bool,
) -> RecordedCommandInfo {
    let words = std::iter::once(&command.name)
        .chain(command.args.iter())
        .collect::<Vec<_>>();
    let mut static_callee =
        static_command_name_text(&command.name, source).map(|name| name.into_owned());
    let static_args = command
        .args
        .iter()
        .map(|word| static_word_text(word, source).map(|text| text.into_owned()))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let source_path_template = static_callee
        .as_deref()
        .filter(|name| matches!(*name, "source" | "."))
        .and_then(|_| command.args.first())
        .and_then(|word| source_path_template(word, source, bash_runtime_vars_enabled));

    if static_callee.as_deref() == Some("noglob") {
        static_callee = words
            .get(1)
            .and_then(|word| static_command_name_text(word, source).map(|name| name.into_owned()));
    }

    let mut info = RecordedCommandInfo {
        static_callee,
        static_args,
        source_path_template,
        zsh_effects: Vec::new(),
    };
    let Some((effect_callee, effect_index)) = normalize_recorded_zsh_effect_command(&words, source)
    else {
        return info;
    };
    let args = words.get(effect_index + 1..).unwrap_or(&[]);

    match effect_callee.as_str() {
        "emulate" => info.zsh_effects = parse_emulate_effects(args, source),
        "setopt" => {
            info.zsh_effects = vec![RecordedZshCommandEffect::SetOptions {
                updates: parse_setopt_updates(args, source, true),
            }];
        }
        "unsetopt" => {
            info.zsh_effects = vec![RecordedZshCommandEffect::SetOptions {
                updates: parse_setopt_updates(args, source, false),
            }];
        }
        "set" => {
            let updates = parse_set_builtin_option_updates(args, source);
            if !updates.is_empty() {
                info.zsh_effects = vec![RecordedZshCommandEffect::SetOptions { updates }];
            }
        }
        _ => {}
    }

    info.zsh_effects.retain(|effect| match effect {
        RecordedZshCommandEffect::Emulate { .. } => true,
        RecordedZshCommandEffect::SetOptions { updates } => !updates.is_empty(),
    });
    info
}

pub(super) fn normalize_recorded_zsh_effect_command(
    words: &[&Word],
    source: &str,
) -> Option<(String, usize)> {
    let mut index = 0usize;

    while let Some(word) = words.get(index) {
        let text = static_word_text(word, source)?;
        if is_recorded_assignment_word(&text) {
            index += 1;
            continue;
        }

        match static_command_wrapper_target_index(words.len(), index, text.as_ref(), |word_index| {
            static_word_text(words[word_index], source)
        }) {
            StaticCommandWrapperTarget::NotWrapper => return Some((text.into_owned(), index)),
            StaticCommandWrapperTarget::Wrapper {
                target_index: Some(target_index),
            } => {
                index = target_index;
                continue;
            }
            StaticCommandWrapperTarget::Wrapper { target_index: None } => return None,
        }
    }

    None
}

pub(super) fn is_recorded_assignment_word(word: &str) -> bool {
    let Some((name, _value)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub(super) fn parse_emulate_effects(args: &[&Word], source: &str) -> Vec<RecordedZshCommandEffect> {
    let mut local = false;
    let mut mode = None;
    let mut updates = Vec::new();
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            index += 1;
            continue;
        };

        match text.as_ref() {
            "--" => {
                break;
            }
            "-o" | "+o" => {
                let enable = text.starts_with('-');
                if let Some(option) = args
                    .get(index + 1)
                    .and_then(|word| static_word_text(word, source))
                    && let Some(update) = parse_recorded_zsh_option_update(&option, enable)
                {
                    updates.push(update);
                }
                index += 2;
                continue;
            }
            _ => {}
        }

        if text.starts_with("-o") || text.starts_with("+o") {
            let enable = text.starts_with('-');
            if let Some(update) = parse_recorded_zsh_option_update(&text[2..], enable) {
                updates.push(update);
            }
            index += 1;
            continue;
        }

        if let Some(flags) = text.strip_prefix('-') {
            for flag in flags.chars() {
                match flag {
                    'L' => local = true,
                    'R' => {}
                    _ => {}
                }
            }
            index += 1;
            continue;
        }

        if mode.is_none() {
            mode = match text.to_ascii_lowercase().as_str() {
                "zsh" => Some(ZshEmulationMode::Zsh),
                "sh" => Some(ZshEmulationMode::Sh),
                "ksh" => Some(ZshEmulationMode::Ksh),
                "csh" => Some(ZshEmulationMode::Csh),
                _ => None,
            };
        }
        index += 1;
    }

    let mut effects = Vec::new();
    if let Some(mode) = mode {
        effects.push(RecordedZshCommandEffect::Emulate { mode, local });
    }
    if !updates.is_empty() {
        effects.push(RecordedZshCommandEffect::SetOptions { updates });
    }
    effects
}

pub(super) fn parse_setopt_updates(
    args: &[&Word],
    source: &str,
    enable: bool,
) -> Vec<RecordedZshOptionUpdate> {
    args.iter()
        .filter_map(|word| static_word_text(word, source))
        .filter(|text| text != "--")
        .filter_map(|text| parse_recorded_zsh_option_update(&text, enable))
        .collect()
}

pub(super) fn parse_set_builtin_option_updates(
    args: &[&Word],
    source: &str,
) -> Vec<RecordedZshOptionUpdate> {
    let mut updates = Vec::new();
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            index += 1;
            continue;
        };

        match text.as_ref() {
            "-o" | "+o" => {
                let enable = text.starts_with('-');
                if let Some(name) = args
                    .get(index + 1)
                    .and_then(|word| static_word_text(word, source))
                    && let Some(update) = parse_recorded_zsh_option_update(&name, enable)
                {
                    updates.push(update);
                }
                index += 2;
            }
            _ if text.starts_with("-o") || text.starts_with("+o") => {
                let enable = text.starts_with('-');
                if let Some(update) = parse_recorded_zsh_option_update(&text[2..], enable) {
                    updates.push(update);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }

    updates
}

pub(super) fn parse_recorded_zsh_option_update(
    name: &str,
    enable: bool,
) -> Option<RecordedZshOptionUpdate> {
    let (normalized, inverted) = normalize_recorded_zsh_option_name(name)?;
    let enable = if inverted { !enable } else { enable };

    if normalized == "localoptions" {
        return Some(RecordedZshOptionUpdate::LocalOptions { enable });
    }

    Some(RecordedZshOptionUpdate::Named {
        name: normalized.into_boxed_str(),
        enable,
    })
}

pub(super) fn normalize_recorded_zsh_option_name(name: &str) -> Option<(String, bool)> {
    let mut normalized = String::with_capacity(name.len());
    for ch in name.chars() {
        if matches!(ch, '_' | '-') {
            continue;
        }
        normalized.push(ch.to_ascii_lowercase());
    }

    if normalized.is_empty() {
        return None;
    }

    if let Some(stripped) = normalized.strip_prefix("no")
        && !stripped.is_empty()
    {
        return Some((stripped.to_string(), true));
    }

    Some((normalized, false))
}
