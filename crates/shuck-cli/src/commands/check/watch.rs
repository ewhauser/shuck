use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use anyhow::{Result, anyhow};
use notify::{RecursiveMode, Watcher, recommended_watcher};

use super::display::print_report;
use super::run::run_check_with_cwd;
use crate::ExitStatus;
use crate::args::CheckCommand;
use crate::config::{
    ConfigArguments, discovered_config_path_for_root, resolve_project_root_for_input,
};
use crate::discover::{DEFAULT_IGNORED_DIR_NAMES, normalize_path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WatchTarget {
    pub(super) watch_path: PathBuf,
    pub(super) watch_paths: Vec<PathBuf>,
    pub(super) recursive: bool,
    pub(super) match_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct WatchPath {
    resolved_path: PathBuf,
    canonical_path: PathBuf,
}

impl WatchTarget {
    pub(super) fn recursive(path: PathBuf) -> Self {
        Self {
            watch_path: path.clone(),
            watch_paths: vec![path.clone()],
            recursive: true,
            match_paths: vec![path],
        }
    }

    pub(super) fn file(path: PathBuf) -> Self {
        let watch_path = path.parent().unwrap_or(&path).to_path_buf();
        Self {
            watch_path: watch_path.clone(),
            watch_paths: vec![watch_path],
            recursive: false,
            match_paths: vec![path],
        }
    }

    fn recursive_mode(&self) -> RecursiveMode {
        if self.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        }
    }

    fn matches_event_path(&self, path: &Path) -> bool {
        if self.recursive {
            self.match_paths
                .iter()
                .any(|match_path| path.starts_with(match_path))
        } else {
            self.match_paths.iter().any(|match_path| match_path == path)
        }
    }

    fn add_match_path(&mut self, path: PathBuf) {
        self.match_paths.push(path);
        self.match_paths.sort();
        self.match_paths.dedup();
    }

    fn add_watch_path(&mut self, path: PathBuf) {
        self.watch_paths.push(path);
        self.watch_paths.sort();
        self.watch_paths.dedup();
    }

    fn merge(&mut self, other: WatchTarget) {
        debug_assert_eq!(self.watch_path, other.watch_path);
        debug_assert_eq!(self.recursive, other.recursive);

        self.watch_paths.extend(other.watch_paths);
        self.watch_paths.sort();
        self.watch_paths.dedup();
        self.match_paths.extend(other.match_paths);
        self.match_paths.sort();
        self.match_paths.dedup();
    }

    fn covers(&self, other: &WatchTarget) -> bool {
        if !self.recursive {
            return false;
        }

        other
            .match_paths
            .iter()
            .all(|path| self.matches_event_path(path))
    }
}
pub(super) fn watch_check(
    args: &CheckCommand,
    config_arguments: &ConfigArguments,
    cwd: &Path,
    cache_root: &Path,
) -> Result<ExitStatus> {
    let watch_targets = collect_watch_targets(&args.paths, config_arguments, cwd)?;
    let (tx, rx) = channel();
    let mut watcher = recommended_watcher(tx)?;
    for target in &watch_targets {
        for watch_path in &target.watch_paths {
            watcher.watch(watch_path, target.recursive_mode())?;
        }
    }

    clear_screen()?;
    print_watch_banner("Starting linter in watch mode...")?;
    let report = run_check_with_cwd(args, config_arguments, cwd, cache_root)?;
    print_report(&report, args.output_format)?;

    loop {
        wait_for_watch_rerun(&rx, cache_root, &watch_targets)?;

        clear_screen()?;
        print_watch_banner("File change detected...")?;
        let report = run_check_with_cwd(args, config_arguments, cwd, cache_root)?;
        print_report(&report, args.output_format)?;
    }
}

pub(super) fn should_clear_screen(stdout_is_terminal: bool) -> bool {
    stdout_is_terminal
}

fn clear_screen() -> Result<()> {
    if !should_clear_screen(io::stdout().is_terminal()) {
        return Ok(());
    }
    clearscreen::clear()?;
    Ok(())
}

fn print_watch_banner(message: &str) -> Result<()> {
    let mut stderr = BufWriter::new(io::stderr().lock());
    writeln!(stderr, "{message}")?;
    stderr.flush()?;
    Ok(())
}

fn effective_check_inputs(paths: &[PathBuf]) -> Vec<PathBuf> {
    if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths.to_vec()
    }
}

pub(super) fn collect_watch_targets(
    paths: &[PathBuf],
    config_arguments: &ConfigArguments,
    cwd: &Path,
) -> Result<Vec<WatchTarget>> {
    let inputs = effective_check_inputs(paths);
    let mut targets = Vec::new();
    for input in inputs {
        let resolved_input = if input.is_absolute() {
            normalize_path(&input)
        } else {
            normalize_path(&cwd.join(&input))
        };
        let metadata = fs::metadata(&resolved_input)?;
        let canonical_input = fs::canonicalize(&resolved_input).map_err(anyhow::Error::from)?;

        let mut target = if metadata.is_dir() {
            WatchTarget::recursive(resolved_input.clone())
        } else {
            WatchTarget::file(resolved_input.clone())
        };
        if metadata.is_dir() {
            target.add_watch_path(canonical_input.clone());
        } else if let Some(parent) = canonical_input.parent() {
            target.add_watch_path(parent.to_path_buf());
        }
        target.add_match_path(canonical_input);
        targets.push(target);

        if let Some(config_path) = watch_config_target(config_arguments, cwd, &resolved_input)? {
            let canonical_config_parent =
                config_path.canonical_path.parent().map(Path::to_path_buf);
            let mut target = WatchTarget::file(config_path.resolved_path);
            target.add_match_path(config_path.canonical_path);
            if let Some(parent) = canonical_config_parent {
                target.add_watch_path(parent.to_path_buf());
            }
            targets.push(target);
        }
    }

    targets.sort_by(|left, right| {
        left.watch_path
            .components()
            .count()
            .cmp(&right.watch_path.components().count())
            .then_with(|| right.recursive.cmp(&left.recursive))
            .then_with(|| left.watch_path.cmp(&right.watch_path))
    });

    let mut deduped = Vec::new();
    for target in targets {
        if let Some(existing) = deduped.iter_mut().find(|existing: &&mut WatchTarget| {
            existing.watch_path == target.watch_path && existing.recursive == target.recursive
        }) {
            existing.merge(target);
            continue;
        }

        if deduped
            .iter()
            .any(|existing: &WatchTarget| existing.covers(&target))
        {
            continue;
        }

        if target.recursive {
            deduped.retain(|existing| !target.covers(existing));
        }

        deduped.push(target);
    }

    Ok(deduped)
}

fn watch_config_target(
    config_arguments: &ConfigArguments,
    cwd: &Path,
    resolved_input: &Path,
) -> Result<Option<WatchPath>> {
    if let Some(explicit_config) = config_arguments.explicit_config_file() {
        let resolved_config = if explicit_config.is_absolute() {
            normalize_path(explicit_config)
        } else {
            normalize_path(&cwd.join(explicit_config))
        };

        return Ok(Some(WatchPath {
            canonical_path: fs::canonicalize(&resolved_config).map_err(anyhow::Error::from)?,
            resolved_path: resolved_config,
        }));
    }

    if !config_arguments.use_config_roots() {
        return Ok(None);
    }

    let project_root = resolve_project_root_for_input(resolved_input, true)?;
    let Some(config_path) = discovered_config_path_for_root(&project_root)? else {
        return Ok(None);
    };

    let resolved_path = normalize_path(&config_path);
    Ok(Some(WatchPath {
        canonical_path: fs::canonicalize(&resolved_path).map_err(anyhow::Error::from)?,
        resolved_path,
    }))
}

fn wait_for_watch_rerun(
    rx: &Receiver<notify::Result<notify::Event>>,
    cache_root: &Path,
    watch_targets: &[WatchTarget],
) -> Result<()> {
    loop {
        let event = match rx.recv() {
            Ok(Ok(event)) => event,
            Ok(Err(error)) => return Err(error.into()),
            Err(error) => return Err(error.into()),
        };

        if drain_watch_batch(event, rx, cache_root, watch_targets)? {
            return Ok(());
        }
    }
}

pub(super) fn drain_watch_batch(
    first_event: notify::Event,
    rx: &Receiver<notify::Result<notify::Event>>,
    cache_root: &Path,
    watch_targets: &[WatchTarget],
) -> Result<bool> {
    let mut should_rerun = watch_event_requires_rerun(&first_event, cache_root, watch_targets);

    loop {
        match rx.try_recv() {
            Ok(Ok(event)) => {
                should_rerun |= watch_event_requires_rerun(&event, cache_root, watch_targets);
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(TryRecvError::Empty) => return Ok(should_rerun),
            Err(TryRecvError::Disconnected) => {
                return Err(anyhow!("watch channel disconnected"));
            }
        }
    }
}

pub(super) fn watch_event_requires_rerun(
    event: &notify::Event,
    cache_root: &Path,
    watch_targets: &[WatchTarget],
) -> bool {
    if event.kind.is_access() || event.kind.is_other() {
        return false;
    }

    if event.need_rescan() {
        return true;
    }

    event
        .paths
        .iter()
        .map(|path| normalize_path(path))
        .filter(|path| !watch_event_path_is_ignored(path, cache_root))
        .any(|path| {
            watch_targets
                .iter()
                .any(|target| target.matches_event_path(&path))
        })
}

fn watch_event_path_is_ignored(path: &Path, cache_root: &Path) -> bool {
    path.starts_with(cache_root)
        || path.components().any(|component| {
            let std::path::Component::Normal(part) = component else {
                return false;
            };
            DEFAULT_IGNORED_DIR_NAMES
                .iter()
                .any(|name| part == OsStr::new(name))
        })
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]

    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::mpsc::{TryRecvError, channel};

    use notify::event::{CreateKind, EventAttributes, ModifyKind, RemoveKind, RenameMode};
    use shuck_extract::{
        EmbeddedFormat, EmbeddedScript, ExtractedDialect, HostLineStart, ImplicitShellFlags,
    };
    use shuck_linter::{
        Category, LinterSettings, Rule, RuleSelector, RuleSet, ShellCheckCodeMap, ShellDialect,
    };
    use shuck_parser::parser::Parser;
    use tempfile::tempdir;

    use super::*;
    use crate::ExitStatus;
    use crate::args::{
        CheckCommand, CheckOutputFormatArg, FileSelectionArgs, PatternRuleSelectorPair,
        PatternShellPair, RuleSelectionArgs,
    };
    use crate::commands::check::add_ignore::run_add_ignore_with_cwd;
    use crate::commands::check::analyze::{
        analyze_file, collect_lint_diagnostics, read_shared_source,
    };
    use crate::commands::check::cache::CachedDisplayedDiagnosticKind;
    use crate::commands::check::display::display_lint_diagnostics;
    use crate::commands::check::embedded::remap_embedded_position;
    use crate::commands::check::run::run_check_with_cwd;
    use crate::commands::check::settings::{
        CompiledPerFileShellList, PerFileShell, parse_rule_selectors,
    };
    use crate::commands::check::test_support::*;
    use crate::commands::check::watch::{
        WatchTarget, collect_watch_targets, drain_watch_batch, should_clear_screen,
        watch_event_requires_rerun,
    };
    use crate::commands::check::{CheckReport, diagnostics_exit_status};
    use crate::commands::check_output::{
        DisplayPosition, DisplaySpan, DisplayedDiagnostic, DisplayedDiagnosticKind, print_report_to,
    };
    use crate::commands::project_runner::PendingProjectFile;
    use crate::config::ConfigArguments;
    use crate::discover::{FileKind, normalize_path};

    #[test]
    fn watch_event_filter_ignores_access_other_ignored_dirs_and_cache_paths() {
        let cache_root = Path::new("/tmp/shuck-cache");
        let watch_targets = vec![
            WatchTarget::recursive(PathBuf::from("/workspace/project")),
            WatchTarget::file(PathBuf::from("/workspace/config/shuck.toml")),
        ];

        assert!(!watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Access(notify::event::AccessKind::Any),
                paths: vec![PathBuf::from("script.sh")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
        assert!(!watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Other,
                paths: vec![PathBuf::from("script.sh")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
        assert!(!watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Create(CreateKind::File),
                paths: vec![PathBuf::from(".git/hooks/post-commit")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
        assert!(!watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Modify(ModifyKind::Data(
                    notify::event::DataChange::Content,
                )),
                paths: vec![cache_root.join("entry.bin")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
        assert!(!watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Modify(ModifyKind::Data(
                    notify::event::DataChange::Content,
                )),
                paths: vec![PathBuf::from("/workspace/config/other.txt")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
    }

    #[test]
    fn watch_event_filter_triggers_on_create_modify_remove_rename_and_rescan() {
        let cache_root = Path::new("/tmp/shuck-cache");
        let watch_targets = vec![
            WatchTarget::recursive(PathBuf::from("/workspace/project")),
            WatchTarget::file(PathBuf::from("/workspace/config/shuck.toml")),
        ];

        assert!(watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Create(CreateKind::File),
                paths: vec![PathBuf::from("/workspace/project/script.sh")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
        assert!(watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Modify(ModifyKind::Data(
                    notify::event::DataChange::Content,
                )),
                paths: vec![PathBuf::from("/workspace/config/shuck.toml")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
        assert!(watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Remove(RemoveKind::File),
                paths: vec![PathBuf::from("/workspace/project/script.sh")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
        assert!(watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                paths: vec![
                    PathBuf::from("/tmp/tempfile"),
                    PathBuf::from("/workspace/config/shuck.toml"),
                ],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));

        let mut attrs = EventAttributes::default();
        attrs.set_flag(notify::event::Flag::Rescan);
        assert!(watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Modify(ModifyKind::Any),
                paths: vec![],
                attrs,
            },
            cache_root,
            &watch_targets,
        ));
    }

    #[test]
    fn clear_screen_requires_terminal_stdout() {
        assert!(should_clear_screen(true));
        assert!(!should_clear_screen(false));
    }

    #[test]
    fn collect_watch_targets_stay_within_requested_scope_and_watch_config_files() {
        let tempdir = tempdir().unwrap();
        let nested = tempdir.path().join("nested");
        let deeper = nested.join("deeper");
        fs::create_dir_all(&deeper).unwrap();
        fs::write(tempdir.path().join("shuck.toml"), "[format]\n").unwrap();
        let file = nested.join("script.sh");
        fs::write(&file, "#!/bin/bash\necho ok\n").unwrap();

        let default_targets =
            collect_watch_targets(&[], &ConfigArguments::default(), tempdir.path()).unwrap();
        assert_eq!(
            default_targets,
            vec![WatchTarget {
                watch_path: normalize_path(tempdir.path()),
                watch_paths: watch_paths(
                    &fs::canonicalize(tempdir.path()).unwrap(),
                    tempdir.path()
                ),
                recursive: true,
                match_paths: match_paths(
                    &fs::canonicalize(tempdir.path()).unwrap(),
                    tempdir.path()
                ),
            }]
        );

        let nested_targets = collect_watch_targets(
            &[PathBuf::from("nested"), PathBuf::from("nested/deeper")],
            &ConfigArguments::default(),
            tempdir.path(),
        )
        .unwrap();
        assert_eq!(
            nested_targets,
            vec![
                WatchTarget {
                    watch_path: normalize_path(tempdir.path()),
                    watch_paths: watch_paths(
                        &fs::canonicalize(tempdir.path()).unwrap(),
                        tempdir.path()
                    ),
                    recursive: false,
                    match_paths: match_paths(
                        &fs::canonicalize(tempdir.path().join("shuck.toml")).unwrap(),
                        &tempdir.path().join("shuck.toml"),
                    ),
                },
                WatchTarget {
                    watch_path: normalize_path(&nested),
                    watch_paths: watch_paths(&fs::canonicalize(&nested).unwrap(), &nested),
                    recursive: true,
                    match_paths: match_paths(&fs::canonicalize(&nested).unwrap(), &nested),
                },
            ]
        );

        let file_targets = collect_watch_targets(
            &[PathBuf::from("nested/script.sh")],
            &ConfigArguments::default(),
            tempdir.path(),
        )
        .unwrap();
        assert_eq!(
            file_targets,
            vec![
                WatchTarget {
                    watch_path: normalize_path(tempdir.path()),
                    watch_paths: watch_paths(
                        &fs::canonicalize(tempdir.path()).unwrap(),
                        tempdir.path()
                    ),
                    recursive: false,
                    match_paths: match_paths(
                        &fs::canonicalize(tempdir.path().join("shuck.toml")).unwrap(),
                        &tempdir.path().join("shuck.toml"),
                    ),
                },
                WatchTarget {
                    watch_path: normalize_path(&nested),
                    watch_paths: watch_paths(&fs::canonicalize(&nested).unwrap(), &nested),
                    recursive: false,
                    match_paths: match_paths(&fs::canonicalize(&file).unwrap(), &file),
                },
            ]
        );
    }

    #[test]
    fn collect_watch_targets_merge_files_in_the_same_parent_directory() {
        let tempdir = tempdir().unwrap();
        let nested = tempdir.path().join("nested");
        fs::create_dir_all(&nested).unwrap();
        let first = nested.join("first.sh");
        let second = nested.join("second.sh");
        fs::write(&first, "#!/bin/bash\necho ok\n").unwrap();
        fs::write(&second, "#!/bin/bash\necho ok\n").unwrap();

        let targets = collect_watch_targets(
            &[
                PathBuf::from("nested/first.sh"),
                PathBuf::from("nested/second.sh"),
            ],
            &ConfigArguments::from_cli(Vec::new(), true).unwrap(),
            tempdir.path(),
        )
        .unwrap();

        assert_eq!(
            targets,
            vec![WatchTarget {
                watch_path: normalize_path(&nested),
                watch_paths: watch_paths(&fs::canonicalize(&nested).unwrap(), &nested),
                recursive: false,
                match_paths: {
                    let mut paths = vec![
                        fs::canonicalize(&first).unwrap(),
                        normalize_path(&first),
                        fs::canonicalize(&second).unwrap(),
                        normalize_path(&second),
                    ];
                    paths.sort();
                    paths.dedup();
                    paths
                },
            }]
        );
    }

    #[test]
    fn drain_watch_batch_coalesces_queued_events_before_rerunning() {
        let cache_root = Path::new("/tmp/shuck-cache");
        let watch_targets = vec![WatchTarget::recursive(PathBuf::from("/workspace/project"))];
        let (tx, rx) = channel();

        tx.send(Ok(notify::Event {
            kind: notify::EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            paths: vec![PathBuf::from("/workspace/project/ignored/.git/index")],
            attrs: EventAttributes::default(),
        }))
        .unwrap();

        let first = notify::Event {
            kind: notify::EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            paths: vec![PathBuf::from("/workspace/project/script.sh")],
            attrs: EventAttributes::default(),
        };

        assert!(drain_watch_batch(first, &rx, cache_root, &watch_targets).unwrap());
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }
}
