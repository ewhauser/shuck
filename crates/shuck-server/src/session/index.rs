#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::anyhow;
use lsp_types::{FileEvent, Url};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::edit::{DocumentKey, DocumentVersion};
use crate::session::settings::{
    ClientSettings, GlobalClientSettings, ResolvedProjectSettings, SettingsResolveContext,
    ShuckSettings,
};
use crate::session::{Client, ClientOptions, WorkspaceOptionsMap};
use crate::workspace::Workspaces;
use crate::{PositionEncoding, TextDocument};

#[derive(Default)]
pub(crate) struct Index {
    documents: FxHashMap<Url, Arc<TextDocument>>,
    workspace_roots: Vec<PathBuf>,
    workspace_settings: Vec<WorkspaceSettings>,
    cache_project_settings: bool,
    project_settings_cache:
        RwLock<FxHashMap<ProjectSettingsCacheKey, Arc<ResolvedProjectSettings>>>,
}

#[derive(Clone)]
struct WorkspaceSettings {
    url: Url,
    root: PathBuf,
    options: Option<ClientOptions>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ProjectSettingsCacheKey {
    config_root: PathBuf,
    workspace_root: Option<PathBuf>,
}

#[derive(Clone)]
pub enum DocumentQuery {
    Text {
        file_url: Url,
        document: Arc<TextDocument>,
        settings: Arc<ShuckSettings>,
    },
}

impl Index {
    pub(super) fn new(
        workspaces: &Workspaces,
        cache_project_settings: bool,
        _global: &GlobalClientSettings,
        _client: &Client,
    ) -> crate::Result<Self> {
        let workspace_settings = workspaces
            .iter()
            .filter_map(|workspace| {
                workspace
                    .url()
                    .to_file_path()
                    .ok()
                    .map(|root| WorkspaceSettings {
                        url: workspace.url().clone(),
                        root,
                        options: workspace.options().cloned(),
                    })
            })
            .collect::<Vec<_>>();
        let workspace_roots = workspace_settings
            .iter()
            .map(|workspace| workspace.root.clone())
            .collect();
        Ok(Self {
            documents: FxHashMap::default(),
            workspace_roots,
            workspace_settings,
            cache_project_settings,
            project_settings_cache: RwLock::new(FxHashMap::default()),
        })
    }

    pub(super) fn key_from_url(&self, url: Url) -> DocumentKey {
        DocumentKey::Text(url)
    }

    pub(super) fn make_document_ref(
        &self,
        key: DocumentKey,
        settings: Arc<crate::session::settings::ShuckSettings>,
    ) -> Option<DocumentQuery> {
        let DocumentKey::Text(url) = key;
        let document = self.documents.get(&url)?.clone();
        Some(DocumentQuery::Text {
            file_url: url,
            document,
            settings,
        })
    }

    pub(super) fn resolve_snapshot_settings(
        &self,
        url: &Url,
        global_options: &ClientOptions,
    ) -> (Arc<ShuckSettings>, Arc<ClientSettings>) {
        let file_path = url.to_file_path().ok();
        let workspace_settings = self.workspace_settings_for_url(url);

        if let Some(workspace_options) =
            workspace_settings.and_then(|workspace| workspace.options.as_ref())
        {
            let option_layers = [global_options, workspace_options];
            return (
                self.resolve_shuck_settings(
                    file_path.as_deref(),
                    workspace_settings.map(|workspace| workspace.root.as_path()),
                    &option_layers,
                ),
                Arc::new(ClientSettings::from_layered_options(&option_layers)),
            );
        }

        let option_layers = [global_options];
        (
            self.resolve_shuck_settings(
                file_path.as_deref(),
                workspace_settings.map(|workspace| workspace.root.as_path()),
                &option_layers,
            ),
            Arc::new(ClientSettings::from_layered_options(&option_layers)),
        )
    }

    pub(super) fn has_open_document(&self, key: &DocumentKey) -> bool {
        let DocumentKey::Text(url) = key;
        self.documents.contains_key(url)
    }

    pub(super) fn update_text_document(
        &mut self,
        key: &DocumentKey,
        content_changes: Vec<lsp_types::TextDocumentContentChangeEvent>,
        new_version: DocumentVersion,
        encoding: PositionEncoding,
    ) -> crate::Result<()> {
        let DocumentKey::Text(url) = key;
        let Some(document) = self.documents.get_mut(url) else {
            return Err(anyhow!(
                "text document URI does not point to an open document"
            ));
        };

        std::sync::Arc::make_mut(document).apply_changes(content_changes, new_version, encoding);
        Ok(())
    }

    pub(super) fn open_text_document(&mut self, url: Url, document: TextDocument) {
        self.documents.insert(url, Arc::new(document));
    }

    pub(super) fn close_document(&mut self, key: &DocumentKey) -> crate::Result<()> {
        let DocumentKey::Text(url) = key;
        self.documents
            .remove(url)
            .map(|_| ())
            .ok_or_else(|| anyhow!("document is not open: {url}"))
    }

    pub(super) fn reload_settings(&mut self, changes: &[FileEvent], _client: &Client) {
        let changed_roots = changes
            .iter()
            .filter_map(|change| change.uri.to_file_path().ok())
            .filter(|path| {
                matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some(".shuck.toml" | "shuck.toml")
                )
            })
            .filter_map(|path| path.parent().map(Path::to_path_buf))
            .collect::<FxHashSet<_>>();
        if changed_roots.is_empty() {
            return;
        }

        self.project_settings_cache
            .write()
            .expect("settings cache lock should not be poisoned")
            .retain(|cache_key, _| !changed_roots.contains(&cache_key.config_root));
    }

    pub(super) fn open_workspace_folder(
        &mut self,
        url: Url,
        _global: &GlobalClientSettings,
        _client: &Client,
    ) -> crate::Result<()> {
        let path = url
            .to_file_path()
            .map_err(|()| anyhow!("failed to convert workspace URL to file path: {url}"))?;
        if !self.workspace_roots.contains(&path) {
            self.workspace_roots.push(path.clone());
        }
        if !self
            .workspace_settings
            .iter()
            .any(|workspace| workspace.root == path)
        {
            self.workspace_settings.push(WorkspaceSettings {
                url,
                root: path,
                options: None,
            });
        }
        self.clear_project_settings_cache();
        Ok(())
    }

    pub(super) fn close_workspace_folder(&mut self, workspace_url: &Url) -> crate::Result<()> {
        let path = workspace_url.to_file_path().map_err(|()| {
            anyhow!("failed to convert workspace URL to file path: {workspace_url}")
        })?;
        self.workspace_roots.retain(|root| root != &path);
        self.workspace_settings
            .retain(|workspace| workspace.root != path);
        self.documents.retain(|url, _| {
            url.to_file_path()
                .map(|file_path| !file_path.starts_with(&path))
                .unwrap_or(true)
        });
        self.clear_project_settings_cache();
        Ok(())
    }

    pub(super) fn config_file_paths(&self) -> impl Iterator<Item = &Path> {
        std::iter::empty()
    }

    pub(super) fn workspace_roots(&self) -> &[PathBuf] {
        &self.workspace_roots
    }

    pub(super) fn workspace_options_for_url(&self, url: &Url) -> Option<&ClientOptions> {
        self.workspace_settings_for_url(url)
            .and_then(|workspace| workspace.options.as_ref())
    }

    pub(super) fn clear_project_settings_cache(&self) {
        self.project_settings_cache
            .write()
            .expect("settings cache lock should not be poisoned")
            .clear();
    }

    fn workspace_settings_for_url(&self, url: &Url) -> Option<&WorkspaceSettings> {
        let path = url.to_file_path().ok()?;
        self.workspace_settings
            .iter()
            .filter(|workspace| path.starts_with(&workspace.root))
            .max_by_key(|workspace| workspace.root.components().count())
    }

    fn resolve_shuck_settings(
        &self,
        file_path: Option<&Path>,
        workspace_root: Option<&Path>,
        option_layers: &[&ClientOptions],
    ) -> Arc<ShuckSettings> {
        if !self.cache_project_settings {
            return Arc::new(ShuckSettings::resolve(
                file_path,
                self.workspace_roots(),
                option_layers,
            ));
        }

        let Some(file_path) = file_path else {
            return Arc::new(ShuckSettings::resolve(
                None,
                self.workspace_roots(),
                option_layers,
            ));
        };

        let context = SettingsResolveContext::for_file(file_path, self.workspace_roots());
        let cache_key = ProjectSettingsCacheKey {
            config_root: context.config_root().to_path_buf(),
            workspace_root: workspace_root.map(Path::to_path_buf),
        };

        if let Some(cached) = self
            .project_settings_cache
            .read()
            .expect("settings cache lock should not be poisoned")
            .get(&cache_key)
            .cloned()
        {
            return Arc::new(cached.for_file(Some(file_path)));
        }

        let resolved = Arc::new(ResolvedProjectSettings::resolve(&context, option_layers));
        let cached = self
            .project_settings_cache
            .write()
            .expect("settings cache lock should not be poisoned")
            .entry(cache_key)
            .or_insert_with(|| resolved.clone())
            .clone();

        Arc::new(cached.for_file(Some(file_path)))
    }

    pub(super) fn update_workspace_options(&mut self, mut workspace_options: WorkspaceOptionsMap) {
        for workspace in &mut self.workspace_settings {
            workspace.options = workspace_options.remove(&workspace.url);
        }
        self.clear_project_settings_cache();
    }

    pub(super) fn open_document_count(&self) -> usize {
        self.documents.len()
    }
}

impl DocumentQuery {
    pub(crate) fn file_url(&self) -> &Url {
        match self {
            Self::Text { file_url, .. } => file_url,
        }
    }

    pub(crate) fn file_path(&self) -> Option<PathBuf> {
        self.file_url().to_file_path().ok()
    }

    pub(crate) fn document(&self) -> &Arc<TextDocument> {
        match self {
            Self::Text { document, .. } => document,
        }
    }

    pub(crate) fn language_id(&self) -> Option<crate::edit::LanguageId> {
        self.document().language_id()
    }

    pub(crate) fn settings(&self) -> &ShuckSettings {
        match self {
            Self::Text { settings, .. } => settings,
        }
    }
}
