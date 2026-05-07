use std::fs;
use std::path::{Path, PathBuf};

use shuck_ast::Name;
use shuck_indexer::Indexer;
use shuck_parser::parser::{Parser, ShellDialect};

use crate::{
    BindingKind, FileContract, PluginFramework, PluginRequest, PluginResolution, PluginResolver,
    SemanticBuildOptions, SemanticModel, ShellProfile, resolve_zsh_plugin_entrypoint,
};

fn zsh_plugin_fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("zsh-plugins")
        .join(name)
}

fn model_at_path_with_plugin_resolver(
    path: &Path,
    plugin_resolver: &(dyn PluginResolver + Send + Sync),
) -> SemanticModel {
    let source = fs::read_to_string(path).unwrap();
    let output = Parser::with_dialect(&source, ShellDialect::Zsh)
        .parse()
        .unwrap();
    let indexer = Indexer::new(&source, &output);
    SemanticModel::build_with_options(
        &output.file,
        &source,
        &indexer,
        SemanticBuildOptions {
            source_path: Some(path),
            plugin_resolver: Some(plugin_resolver),
            shell_profile: Some(ShellProfile::native(shuck_parser::ShellDialect::Zsh)),
            ..SemanticBuildOptions::default()
        },
    )
}

struct FixtureZshPluginResolver {
    roots: Vec<(PluginFramework, PathBuf)>,
}

impl FixtureZshPluginResolver {
    fn new(roots: Vec<(PluginFramework, PathBuf)>) -> Self {
        Self { roots }
    }
}

impl PluginResolver for FixtureZshPluginResolver {
    fn resolve_plugin_request(
        &self,
        _source_path: &Path,
        request: &PluginRequest,
    ) -> PluginResolution {
        let Some((_, root)) = self
            .roots
            .iter()
            .find(|(framework, _)| framework == &request.framework)
        else {
            return PluginResolution::default();
        };
        let Some(entrypoint) = resolve_zsh_plugin_entrypoint(root, request) else {
            return PluginResolution::default();
        };
        PluginResolution {
            entrypoints: vec![entrypoint],
            file_entry_contracts: Vec::new(),
            requesting_file_contract: FileContract::default(),
        }
    }
}

fn reportable_unused_names(model: &SemanticModel) -> Vec<Name> {
    let analysis = model.analysis();
    analysis
        .unused_assignments()
        .iter()
        .filter_map(|binding| {
            let binding = model.binding(*binding);
            matches!(
                binding.kind,
                BindingKind::Assignment
                    | BindingKind::ArrayAssignment
                    | BindingKind::LoopVariable
                    | BindingKind::ReadTarget
                    | BindingKind::MapfileTarget
                    | BindingKind::PrintfTarget
                    | BindingKind::GetoptsTarget
                    | BindingKind::ArithmeticAssignment
            )
            .then_some(binding.name.clone())
        })
        .collect()
}

fn assert_autosuggest_strategy_was_consumed(model: &SemanticModel) {
    assert!(
        model
            .synthetic_reads
            .iter()
            .any(|read| read.name == "ZSH_AUTOSUGGEST_STRATEGY"),
        "synthetic reads: {:?}",
        model.synthetic_reads
    );
    let unused = reportable_unused_names(model);
    assert!(
        !unused.contains(&Name::from("ZSH_AUTOSUGGEST_STRATEGY")),
        "unused: {:?}",
        unused
    );
}

#[test]
fn prezto_autosuggestions_module_loads_standalone_plugin_dependency() {
    let fixture = zsh_plugin_fixture_path("prezto-autosuggestions");
    let resolver = FixtureZshPluginResolver::new(vec![
        (PluginFramework::Prezto, fixture.join("prezto")),
        (
            PluginFramework::Other("zsh-autosuggestions".to_owned()),
            fixture.join("zsh-autosuggestions"),
        ),
    ]);
    let model = model_at_path_with_plugin_resolver(&fixture.join("home/.zpreztorc"), &resolver);

    assert_autosuggest_strategy_was_consumed(&model);
}

#[test]
fn zdot_use_plugin_loads_standalone_plugin_entrypoints() {
    let fixture = zsh_plugin_fixture_path("zdot-use-plugin");
    let resolver = FixtureZshPluginResolver::new(vec![(
        PluginFramework::Other("zsh-autosuggestions".to_owned()),
        fixture.join("zsh-autosuggestions"),
    )]);
    let model = model_at_path_with_plugin_resolver(&fixture.join("home/.zshrc"), &resolver);

    assert_autosuggest_strategy_was_consumed(&model);
}
