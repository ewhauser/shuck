//! zsh ecosystem path classifiers shared by zsh ambient providers.
//!
//! These functions encode repository layout conventions for common zsh runtimes
//! and config trees. For example, files under `plugins/`, `functions/`, or
//! `.zshrc`-style paths are treated differently from arbitrary `*.zsh` scripts.

use super::path::{path_file_name, path_has_component, path_matches_any};

pub(super) fn p10k_config_path_shape(lower_path: &str) -> bool {
    let file_name = path_file_name(lower_path);
    if file_name == ".p10k.zsh"
        || file_name == "p10k.zsh"
        || (file_name.starts_with("p10k-") && file_name.ends_with(".zsh"))
    {
        return true;
    }

    path_matches_any(
        lower_path,
        &[
            "/powerlevel10k/config/",
            "/powerlevel10k/internal/configure.zsh",
        ],
    )
}

pub(super) fn p10k_gitstatus_path_shape(lower_path: &str) -> bool {
    lower_path.contains("/powerlevel10k/gitstatus/")
}

pub(super) fn zsh_dotfile_path_shape(lower_path: &str) -> bool {
    path_has_component(
        lower_path,
        &[
            ".zshrc",
            "zshrc",
            ".zshenv",
            "zshenv",
            ".zprofile",
            "zprofile",
            ".zlogin",
            "zlogin",
            ".zlogout",
            "zlogout",
            "zdot",
        ],
    ) || lower_path.contains("/zsh/config/")
        || lower_path.contains("/zsh/configs/")
}

pub(super) fn zsh_runtime_path_shape(lower_path: &str) -> bool {
    zsh_dotfile_path_shape(lower_path)
        || path_matches_any(
            lower_path,
            &[
                "/completion/",
                "/completions/",
                "/functions/",
                "/highlighters/",
                "/lib/",
                "/modules/",
                "/plugins/",
                "/plugin/",
                "/themes/",
                ".plugin.zsh",
                ".theme.zsh",
                "/zsh-autosuggestions/",
                "/zsh-syntax-highlighting/",
            ],
        )
}

pub(super) fn zsh_project_path_shape(lower_path: &str) -> bool {
    path_matches_any(
        lower_path,
        &[
            "/ohmyzsh/",
            "/powerlevel10k/",
            "/prezto/",
            "/zinit/",
            "/zsh-autosuggestions/",
            "/zsh-syntax-highlighting/",
        ],
    )
}

pub(super) fn zsh_test_data_path_shape(lower_path: &str) -> bool {
    path_matches_any(lower_path, &["/test-data/", "/testdata/", "/fixtures/"])
}

pub(super) fn zsh_syntax_highlighting_test_path_shape(lower_path: &str) -> bool {
    lower_path.contains("/zsh-syntax-highlighting/")
        && (zsh_test_data_path_shape(lower_path) || lower_path.contains("/tests/"))
}
