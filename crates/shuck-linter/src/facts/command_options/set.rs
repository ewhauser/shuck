use super::*;

pub(super) fn parse_set_command(args: &[&Word], source: &str) -> SetCommandFacts {
    let mut errexit_change = None;
    let mut errtrace_change = None;
    let mut functrace_change = None;
    let mut pipefail_change = None;
    let mut resets_positional_parameters = false;
    let mut errtrace_flag_spans = Vec::new();
    let mut functrace_flag_spans = Vec::new();
    let mut pipefail_option_spans = Vec::new();
    let mut non_posix_option_spans = Vec::new();
    let mut flags_without_prefix_spans = Vec::new();
    let mut index = 0usize;

    if args.len() >= 2
        && let Some(first_word) = args.first().copied()
        && classify_word(first_word, source).quote == WordQuote::Unquoted
        && let Some(first_text) = static_word_text(first_word, source)
        && first_text != "--"
        && !first_text.starts_with('-')
        && !first_text.starts_with('+')
        && is_shell_variable_name(first_text.as_ref())
    {
        flags_without_prefix_spans.push(first_word.span);
    }

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            resets_positional_parameters = true;
            break;
        };

        if text == "--" {
            resets_positional_parameters = true;
            break;
        }

        match text.as_ref() {
            "-o" | "+o" => {
                let enable = text.starts_with('-');
                if text.starts_with('+') {
                    resets_positional_parameters = true;
                }
                let Some(name_word) = args.get(index + 1) else {
                    break;
                };
                let Some(name) = static_word_text(name_word, source) else {
                    break;
                };

                if name == "errexit" {
                    errexit_change = Some(enable);
                } else if name == "errtrace" {
                    errtrace_change = Some(enable);
                } else if name == "functrace" {
                    functrace_change = Some(enable);
                } else if name == "pipefail" {
                    pipefail_change = Some(enable);
                    pipefail_option_spans.push(name_word.span);
                }

                if !set_o_option_name_is_posix(&name) {
                    non_posix_option_spans.push(name_word.span);
                }
                index += 2;
                continue;
            }
            _ => {}
        }

        let Some(flags) = text.strip_prefix('-').or_else(|| text.strip_prefix('+')) else {
            resets_positional_parameters = true;
            break;
        };
        if flags.is_empty() {
            break;
        }
        if text.starts_with('+') {
            resets_positional_parameters = true;
        }

        if flags.chars().any(|flag| flag == 'e') {
            errexit_change = Some(text.starts_with('-'));
        }
        if flags.chars().any(|flag| flag == 'E') {
            errtrace_change = Some(text.starts_with('-'));
            errtrace_flag_spans.push(word.span);
        }
        if flags.chars().any(|flag| flag == 'T') {
            functrace_change = Some(text.starts_with('-'));
            functrace_flag_spans.push(word.span);
        }

        if flags.chars().any(|flag| flag == 'o') {
            let enable = text.starts_with('-');
            let Some(name_word) = args.get(index + 1) else {
                break;
            };
            let Some(name) = static_word_text(name_word, source) else {
                break;
            };

            if name == "errexit" {
                errexit_change = Some(enable);
            } else if name == "errtrace" {
                errtrace_change = Some(enable);
            } else if name == "functrace" {
                functrace_change = Some(enable);
            } else if name == "pipefail" {
                pipefail_change = Some(enable);
                pipefail_option_spans.push(name_word.span);
            }

            if !set_o_option_name_is_posix(&name) {
                non_posix_option_spans.push(name_word.span);
            }
            index += 2;
            continue;
        }

        index += 1;
    }

    SetCommandFacts {
        errexit_change,
        errtrace_change,
        functrace_change,
        pipefail_change,
        resets_positional_parameters,
        errtrace_flag_spans: errtrace_flag_spans.into_boxed_slice(),
        functrace_flag_spans: functrace_flag_spans.into_boxed_slice(),
        pipefail_option_spans: pipefail_option_spans.into_boxed_slice(),
        non_posix_option_spans: non_posix_option_spans.into_boxed_slice(),
        flags_without_prefix_spans: flags_without_prefix_spans.into_boxed_slice(),
    }
}

fn set_o_option_name_is_posix(name: &str) -> bool {
    matches!(
        name,
        "allexport"
            | "errexit"
            | "ignoreeof"
            | "monitor"
            | "noclobber"
            | "noexec"
            | "noglob"
            | "nolog"
            | "notify"
            | "nounset"
            | "verbose"
            | "vi"
            | "xtrace"
    )
}
