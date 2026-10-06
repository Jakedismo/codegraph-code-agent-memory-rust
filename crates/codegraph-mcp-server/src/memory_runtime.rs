// ABOUTME: Composes existing providers, durable owner IPC, and optional current code grounding.
// ABOUTME: User memory pins its embedding settings independently of calling project configuration.
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use codegraph_core::config_manager::CodeGraphConfig;
use codegraph_graph::GraphFunctions;
use codegraph_memory::{
    Embedder, MemoryService,
    identity::project_identity,
    owner::{self, Action, OwnerClient, Registration, ServiceFactory},
    store::{Store, content_hash, digest},
    types::*,
};
use codegraph_vector::{EmbeddingGenerator, reranking::factory::create_reranker};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Serialize, Deserialize)]
struct Settings {
    config: CodeGraphConfig,
    runtime_controls: String,
    #[cfg(feature = "ai-enhanced")]
    llm: Option<codegraph_mcp_rig::memory::ResolvedLlm>,
}
fn runtime_controls() -> Result<String> {
    // The existing provider pipeline still resolves these process-level controls. Detect
    // a client/owner mismatch rather than silently initialize with another project's
    // tokenizer, prefixes, tasks or endpoint. Ordinary provider/model settings are explicit.
    let mut controls = std::collections::BTreeMap::new();
    for key in [
        "CODEGRAPH_TOKENIZER_PATH",
        "CODEGRAPH_TOKENIZER_REPO",
        "CODEGRAPH_TOKENIZER_REVISION",
        "CODEGRAPH_MODEL_REVISION",
        "CODEGRAPH_MODEL_MAX_TOKENS",
        "CODEGRAPH_OLLAMA_NUM_CTX",
        "CODEGRAPH_EMBEDDING_DOCUMENT_PREFIX",
        "CODEGRAPH_EMBEDDING_QUERY_PREFIX",
        "CODEGRAPH_EMBEDDING_SKIP_CHUNKING",
        "CODEGRAPH_CHUNK_MAX_TOKENS",
        "CODEGRAPH_MAX_CHUNK_TOKENS",
        "CODEGRAPH_EMBEDDING_DIMENSION",
        "CODEGRAPH_ONNX_MODEL_FILE",
        "OPENAI_API_BASE",
        "OPENAI_BASE_URL",
        "JINA_API_TASK",
        "JINA_TASK",
        "JINA_NORMALIZED",
        "JINA_MAX_TOKENS",
        "JINA_MAX_TEXTS",
        "JINA_REL_BATCH_SIZE",
        "JINA_REL_MAX_TEXTS",
    ] {
        let value = nonempty_env(key).map(|value| {
            if matches!(
                key,
                "CODEGRAPH_TOKENIZER_PATH" | "CODEGRAPH_ONNX_MODEL_FILE"
            ) {
                PathBuf::from(&value)
                    .canonicalize()
                    .map_or(value, |path| path.to_string_lossy().into_owned())
            } else {
                value
            }
        });
        controls.insert(key, value);
    }
    digest(&controls)
}
struct Factory;
struct MemoryEmbeddings {
    generator: EmbeddingGenerator,
    identity: String,
}
#[async_trait]
impl Embedder for MemoryEmbeddings {
    fn identity(&self) -> Result<String> {
        Ok(self.identity.clone())
    }
    async fn document(&self, text: &str) -> Result<Vec<Vec<f32>>> {
        self.generator.document(text).await
    }
    async fn query(&self, text: &str) -> Result<Vec<f32>> {
        self.generator.query(text).await
    }
}
type Components = (
    Arc<dyn Embedder>,
    Option<Arc<dyn codegraph_memory::Classifier>>,
    Option<Arc<dyn codegraph_vector::reranking::Reranker>>,
);
impl Factory {
    async fn components(&self, settings: Settings) -> Result<Components> {
        ensure!(
            settings.runtime_controls == runtime_controls()?,
            "Client embedding runtime controls differ from the memory owner; stop the owner and reconnect with the intended tokenizer/task/endpoint settings"
        );
        static CACHE: std::sync::OnceLock<
            tokio::sync::Mutex<std::collections::BTreeMap<String, Components>>,
        > = std::sync::OnceLock::new();
        let key = digest(&settings)?;
        let mut cache = CACHE.get_or_init(Default::default).lock().await;
        if let Some(components) = cache.get(&key) {
            return Ok(components.clone());
        }
        let mut generator = EmbeddingGenerator::with_config(&settings.config).await?;
        generator.enforce_untruncated_inputs();
        ensure!(
            generator.has_provider(),
            "Memory requires a configured embedding provider supported by this build"
        );
        let reranker = create_reranker(&settings.config.rerank)?;
        let classifier: Option<Arc<dyn codegraph_memory::Classifier>> = {
            #[cfg(feature = "ai-enhanced")]
            {
                settings
                    .llm
                    .map(codegraph_mcp_rig::memory::RigMemoryClassifier::new)
                    .transpose()?
                    .map(|v| Arc::new(v) as Arc<dyn codegraph_memory::Classifier>)
            }
            #[cfg(not(feature = "ai-enhanced"))]
            {
                None
            }
        };
        let identity = digest(&(
            "memory-document-input-v1",
            generator.input_identity()?,
            generator.dimension(),
            &settings.config.embedding.provider,
            &settings.config.embedding.ollama_url,
            &settings.config.embedding.lmstudio_url,
            &settings.config.embedding.jina_api_base,
            nonempty_env("OPENAI_API_BASE").or_else(|| nonempty_env("OPENAI_BASE_URL")),
        ))?;
        let components = (
            Arc::new(MemoryEmbeddings {
                generator,
                identity,
            }) as Arc<dyn Embedder>,
            classifier,
            reranker,
        );
        cache.insert(key, components.clone());
        Ok(components)
    }
}
struct UnavailableEmbeddings;
#[async_trait]
impl Embedder for UnavailableEmbeddings {
    fn available(&self) -> bool {
        false
    }
    fn identity(&self) -> Result<String> {
        anyhow::bail!("Compatible embedding configuration must be registered")
    }
    async fn document(&self, _: &str) -> Result<Vec<Vec<f32>>> {
        anyhow::bail!("Compatible embedding configuration must be registered")
    }
    async fn query(&self, _: &str) -> Result<Vec<f32>> {
        anyhow::bail!("Compatible embedding configuration must be registered")
    }
}
#[async_trait]
impl ServiceFactory for Factory {
    async fn create(&self, registration: Registration) -> Result<MemoryService> {
        let settings: Settings = serde_json::from_value(registration.settings)?;
        let (generator, classifier, reranker) = self.components(settings).await?;
        let store = Store::open(Some(&registration.store_path)).await?;
        Ok(MemoryService::new(store, generator, classifier, reranker))
    }
    async fn configure(
        &self,
        existing: MemoryService,
        registration: Registration,
    ) -> Result<MemoryService> {
        let settings: Settings = serde_json::from_value(registration.settings)?;
        let (embedder, classifier, reranker) = self.components(settings).await?;
        let store = existing.store.lock().await;
        ensure!(
            store
                .state
                .embedding_identity
                .as_ref()
                .is_none_or(|v| embedder.identity().is_ok_and(|i| *v == i)),
            "Registered provider identity is incompatible with this memory store; explicit re-embedding is required"
        );
        drop(store);
        Ok(MemoryService {
            store: existing.store,
            embedder,
            classifier,
            reranker,
        })
    }
    async fn recover(&self, path: PathBuf) -> Result<MemoryService> {
        let mut store = Store::open(Some(&path)).await?;
        let mut state = store.state.clone();
        for job in state.operations.values_mut() {
            if matches!(job.state, JobState::Running | JobState::Pending) {
                job.state = JobState::ConfigRequired;
                job.lease_until = None;
                job.error = Some("Register compatible provider configuration to resume".into());
            }
        }
        store.commit(state).await?;
        Ok(MemoryService::new(
            store,
            Arc::new(UnavailableEmbeddings),
            None,
            None,
        ))
    }
    async fn reembed(
        &self,
        existing: MemoryService,
        registration: Registration,
    ) -> Result<MemoryService> {
        let settings: Settings = serde_json::from_value(registration.settings)?;
        let (embedder, classifier, reranker) = self.components(settings).await?;
        existing.reembed(embedder.clone()).await?;
        Ok(MemoryService {
            store: existing.store,
            embedder,
            classifier,
            reranker,
        })
    }
}
pub fn runtime_dir() -> Result<PathBuf> {
    Ok(std::env::var_os("CODEGRAPH_MEMORY_HOME")
        .map(PathBuf::from)
        .unwrap_or(
            dirs::home_dir()
                .context("Cannot locate memory home")?
                .join(".codegraph"),
        )
        .join("memory-runtime"))
}
pub async fn run_owner(path: &Path) -> Result<()> {
    owner::run(path, Arc::new(Factory)).await
}

#[derive(Clone)]
pub struct MemoryRuntime {
    client: OwnerClient,
    pub context: ClientContext,
    project_store: PathBuf,
    user_store: PathBuf,
    settings: Settings,
    graph: Option<Arc<GraphFunctions>>,
    root: PathBuf,
}
impl MemoryRuntime {
    pub async fn connect(
        config: CodeGraphConfig,
        graph: Option<Arc<GraphFunctions>>,
    ) -> Result<Self> {
        Self::connect_inner(config, graph, true).await
    }
    pub async fn connect_for_reembed(config: CodeGraphConfig) -> Result<Self> {
        Self::connect_inner(config, None, false).await
    }
    async fn connect_inner(
        config: CodeGraphConfig,
        graph: Option<Arc<GraphFunctions>>,
        register: bool,
    ) -> Result<Self> {
        let root = std::env::current_dir()?.canonicalize()?;
        let graph = if graph.is_none()
            && (root.join(".codegraph/db").exists()
                || std::env::var_os("CODEGRAPH_SURREALDB_URL").is_some())
        {
            match tokio::time::timeout(
                std::time::Duration::from_secs(1),
                codegraph_graph::SurrealDbStorage::new(
                    codegraph_graph::SurrealDbConfig::for_project(&root),
                ),
            )
            .await
            {
                Ok(Ok(storage)) => Some(Arc::new(GraphFunctions::new_with_project_id(
                    storage.db(),
                    nonempty_env("CODEGRAPH_PROJECT_ID")
                        .unwrap_or_else(|| root.display().to_string()),
                ))),
                _ => None,
            }
        } else {
            graph
        };
        let identity = project_identity(&root)?;
        let home = runtime_dir()?
            .parent()
            .context("Memory home missing")?
            .to_path_buf();
        let client = OwnerClient::connect(&runtime_dir()?, &std::env::current_exe()?).await?;
        let settings = Settings {
            config,
            runtime_controls: runtime_controls()?,
            #[cfg(feature = "ai-enhanced")]
            llm: codegraph_mcp_rig::memory::ResolvedLlm::current().ok(),
        };
        let context = ClientContext {
            // A private per-user owner and its private bearer token establish OS-user authority.
            owner_id: home.canonicalize()?.to_string_lossy().into_owned(),
            project_id: identity.id,
            session_id: nonempty_env("CODEGRAPH_MEMORY_SESSION_ID"),
            task_id: nonempty_env("CODEGRAPH_MEMORY_TASK_ID"),
            context_epoch: nonempty_env("CODEGRAPH_MEMORY_CONTEXT_EPOCH"),
            applicability: Default::default(),
        };
        let runtime = Self {
            client,
            context,
            project_store: identity.store_root,
            user_store: home.join("user-memory-db"),
            settings,
            graph,
            root,
        };
        if register {
            runtime
                .register(&runtime.project_store, &runtime.settings)
                .await?;
        }
        Ok(runtime)
    }
    pub async fn reembed(&self, scope: Scope) -> Result<serde_json::Value> {
        let path = if scope == Scope::User {
            self.user_store.clone()
        } else {
            self.project_store.clone()
        };
        let value = self
            .client
            .request(
                path.clone(),
                self.context.clone(),
                Action::Reembed(Registration {
                    store_path: path,
                    settings: serde_json::to_value(&self.settings)?,
                }),
            )
            .await?;
        ensure!(
            value["memory_schema_version"] == 2,
            "Running memory owner predates native schema v2; run 'codegraph memory service stop' and reconnect with this binary"
        );
        if scope == Scope::User {
            let mut embedding = self.settings.config.embedding.clone();
            embedding.jina_api_key = None;
            embedding.openai_api_key = None;
            std::fs::write(
                self.user_store.with_file_name("user-memory-embedding.json"),
                serde_json::to_vec(&embedding)?,
            )?;
        }
        Ok(value)
    }
    async fn register(&self, path: &Path, settings: &Settings) -> Result<()> {
        let response = self
            .client
            .request(
                path.to_path_buf(),
                self.context.clone(),
                Action::Register(Registration {
                    store_path: path.to_path_buf(),
                    settings: serde_json::to_value(settings)?,
                }),
            )
            .await?;
        ensure!(
            response["memory_schema_version"] == 2,
            "Running memory owner predates native schema v2; run 'codegraph memory service stop' and reconnect with this binary"
        );
        Ok(())
    }
    async fn user_settings(&self) -> Result<Settings> {
        let path = self.user_store.with_file_name("user-memory-embedding.json");
        let lock_path = path.with_extension("lock");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        lock.lock()?;
        let mut settings = self.settings.clone();
        if path.exists() {
            settings.config.embedding = serde_json::from_slice(&std::fs::read(&path)?)?;
        } else {
            let mut embedding = self.settings.config.embedding.clone();
            embedding.jina_api_key = None;
            embedding.openai_api_key = None;
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            use std::io::Write;
            file.write_all(&serde_json::to_vec(&embedding)?)?;
            file.sync_all()?;
        }
        // Supply credentials only for the pinned provider; never persist them.
        if settings.config.embedding.provider == self.settings.config.embedding.provider {
            settings.config.embedding.jina_api_key =
                self.settings.config.embedding.jina_api_key.clone();
            settings.config.embedding.openai_api_key =
                self.settings.config.embedding.openai_api_key.clone();
        }
        drop(lock);
        Ok(settings)
    }
    async fn target(&self, scope: Scope) -> Result<PathBuf> {
        if scope == Scope::User {
            self.register(&self.user_store, &self.user_settings().await?)
                .await?;
            Ok(self.user_store.clone())
        } else {
            Ok(self.project_store.clone())
        }
    }
    pub async fn write(&self, mut request: WriteRequest) -> Result<serde_json::Value> {
        if let Some(graph) = &self.graph {
            let mut anchors = Vec::new();
            if !request.anchors.is_empty() {
                let ids = request
                    .anchors
                    .iter()
                    .map(|v| v.node_id.clone())
                    .collect::<Vec<_>>();
                if let Ok(context) = graph.memory_code_context(&ids).await {
                    anchors = self.anchors(&context, false)?;
                    for anchor in &mut anchors {
                        anchor.supporting = request.anchors.iter().any(|v| {
                            v.supporting
                                && v.node_id.trim_start_matches("nodes:")
                                    == anchor.node_id.trim_start_matches("nodes:")
                        });
                    }
                }
            } else if request.code_related
                && let Ok(anchors_found) = self.find_anchors(&request.statement).await
            {
                anchors = anchors_found;
            }
            request.anchors = anchors;
        } else {
            request.anchors.clear();
        }
        let path = self.target(request.scope).await?;
        self.client
            .request(path, self.context.clone(), Action::Write(request))
            .await
    }
    async fn sync_projection(&self, graph: &GraphFunctions) -> Result<(String, bool)> {
        use codegraph_core::memory_projection::{ProjectionBatch, ProjectionCursor};
        let source = codegraph_memory::store::digest(&(
            &self.context.owner_id,
            &self.context.project_id,
            graph.project_id(),
        ))?;
        let mut cursor: ProjectionCursor = graph.memory_projection_cursor(&source).await?;
        for _ in 0..8 {
            let value = self
                .client
                .request(
                    self.project_store.clone(),
                    self.context.clone(),
                    Action::ProjectionChanges {
                        graph_project_id: graph.project_id().into(),
                        cursor: cursor.clone(),
                    },
                )
                .await?;
            let batch: ProjectionBatch = serde_json::from_value(value)?;
            ensure!(
                batch.source_id == source,
                "Memory projection source mismatch"
            );
            graph.apply_memory_projection(&batch, &cursor).await?;
            cursor = batch.cursor;
            if batch.complete {
                return Ok((source, true));
            }
        }
        Ok((source, false))
    }
    pub async fn read(&self, mut request: ReadRequest) -> Result<MemoryContext> {
        if request.node_ids.is_empty()
            && self.graph.is_some()
            && let Ok(anchors) = self.find_anchors(&request.query).await
        {
            request.node_ids = anchors.into_iter().map(|v| v.node_id).collect();
        }
        let mut sources = Vec::new();
        let mut warnings = Vec::new();
        let mut projection_candidates = Vec::new();
        if request.scope != Some(Scope::User)
            && let Some(graph) = &self.graph
        {
            match tokio::time::timeout(
                std::time::Duration::from_secs(2),
                self.sync_projection(graph),
            )
            .await
            {
                Ok(Ok((source, complete))) => {
                    if !complete {
                        warnings.push("Code-side memory projection is partially synchronized; semantic recall remains available".into());
                    }
                    match graph
                        .memory_projection_candidates(&source, &request.node_ids)
                        .await
                    {
                        Ok(candidates) => {
                            for candidate in &candidates {
                                if let (Some(id), Some(revision)) = (
                                    candidate["memory_id"].as_str(),
                                    candidate["revision"].as_u64(),
                                ) {
                                    request
                                        .graph_memory_refs
                                        .push(format!("memory:{id}@{revision}"));
                                }
                            }
                            request.graph_memory_refs.sort();
                            request.graph_memory_refs.dedup();
                            if request.graph_memory_refs.len() > 100 {
                                request.graph_memory_refs.truncate(100);
                                warnings.push(
                                    "Graph-side memory candidates truncated at 100 references"
                                        .into(),
                                );
                            }
                            projection_candidates = candidates;
                        }
                        Err(error) => {
                            warnings.push(format!("Code-side memory joins unavailable: {error}"))
                        }
                    }
                }
                Ok(Err(error)) => warnings.push(format!(
                    "Code-side memory projection unavailable: {error:#}"
                )),
                Err(_) => warnings.push(
                    "Code-side memory projection timed out; semantic recall remains available"
                        .into(),
                ),
            }
        }
        let mut project_request = request.clone();
        project_request.limit = 100;
        project_request.token_budget = 100_000;
        if request.scope != Some(Scope::User) {
            match self
                .client
                .request(
                    self.project_store.clone(),
                    self.context.clone(),
                    Action::Read(project_request.clone()),
                )
                .await
            {
                Ok(value) => sources.push((
                    self.project_store.clone(),
                    serde_json::from_value::<MemoryContext>(value)?,
                )),
                Err(error) => warnings.push(format!("Project memory unavailable: {error:#}")),
            }
        }
        if request.scope.is_none() || request.scope == Some(Scope::User) {
            match self.target(Scope::User).await {
                Ok(path) => {
                    let mut user_request = project_request.clone();
                    user_request.scope = Some(Scope::User);
                    user_request.graph_memory_refs.clear();
                    match self
                        .client
                        .request(
                            path.clone(),
                            self.context.clone(),
                            Action::Read(user_request),
                        )
                        .await
                    {
                        Ok(value) => sources.push((path, serde_json::from_value(value)?)),
                        Err(error) => warnings.push(format!("User memory unavailable: {error:#}")),
                    }
                }
                Err(error) => warnings.push(format!("User memory unavailable: {error:#}")),
            }
        }
        let mut entries = Vec::new();
        let mut operation = None;
        let available_sources = sources.len();
        let mut grounding_attempts = 0;
        for (path, source) in sources {
            warnings.extend(
                source
                    .warnings
                    .into_iter()
                    .filter(|v| !v.contains("tokenizer")),
            );
            if source.operation.is_some() {
                operation = source.operation;
            }
            let mut checked = Vec::new();
            let mut source_entries: Vec<_> = source
                .memories
                .into_iter()
                .chain(source.needs_verification)
                .collect();
            for entry in &mut source_entries {
                if path == self.project_store {
                    for candidate in &projection_candidates {
                        if candidate["memory_id"] == entry.memory.id
                            && candidate["revision"] == entry.memory.revision
                        {
                            entry.code_context.truncated |= candidate["paths_truncated"] == true;
                            if let Some(paths) = candidate["paths"].as_array() {
                                entry.code_context.paths.extend(paths.iter().cloned());
                            }
                        }
                    }
                }
                if let Some(graph) = &self.graph {
                    if entry.memory.grounding == Grounding::Unavailable
                        && entry.memory.anchors.is_empty()
                        && grounding_attempts < 3
                    {
                        grounding_attempts += 1;
                        if let Ok(anchors) = self.find_anchors(&entry.memory.statement).await
                            && !anchors.is_empty()
                        {
                            entry.memory.anchors = anchors;
                            entry.memory.grounding = Grounding::Anchored;
                        }
                    }
                    if !entry.memory.anchors.is_empty() {
                        let ids = entry
                            .memory
                            .anchors
                            .iter()
                            .filter(|v| v.graph_project_id == graph.project_id())
                            .map(|v| v.node_id.clone())
                            .collect::<Vec<_>>();
                        match graph.memory_code_context(&ids).await {
                            Ok(context)
                                if !ids.is_empty()
                                    && context["input_fingerprint"].is_string()
                                    && context["graph_complete"] != false =>
                            {
                                self.enrich(entry, &context)?
                            }
                            Ok(_) => {
                                entry.memory.grounding = Grounding::Unavailable;
                                warnings.push("Code anchors cannot be verified against this project's ready index".into());
                            }
                            Err(_) => {
                                entry.memory.grounding = Grounding::Unavailable;
                                warnings.push("Code grounding temporarily unavailable".into());
                            }
                        }
                    }
                }
                checked.push(GroundingCheck {
                    memory_id: entry.memory.id.clone(),
                    expected_revision: entry.memory.revision,
                    anchors: entry.memory.anchors.clone(),
                    grounding: entry.memory.grounding,
                    reasons: entry.reasons.clone(),
                });
            }
            self.client
                .request(path, self.context.clone(), Action::GroundingChecks(checked))
                .await?;
            entries.push(source_entries);
        }
        // Interleave source rankings: extra retrieval streams cannot multiply a store's weight.
        let mut combined = Vec::new();
        let max = entries.iter().map(Vec::len).max().unwrap_or(0);
        for rank in 0..max {
            for source in &entries {
                if let Some(entry) = source.get(rank) {
                    combined.push(entry.clone());
                }
            }
        }
        if combined.is_empty() && !warnings.is_empty() && available_sources == 0 {
            return Ok(MemoryContext::unavailable(warnings.join("; ")));
        }
        let model = {
            #[cfg(feature = "ai-enhanced")]
            {
                self.settings.llm.as_ref().map(|v| v.model.as_str())
            }
            #[cfg(not(feature = "ai-enhanced"))]
            {
                None
            }
        };
        let counter = codegraph_memory::context::consuming_counter(model)?;
        let partial = !warnings.is_empty();
        let mut result = codegraph_memory::context::pack_with_metadata(
            combined,
            request.limit,
            request.token_budget,
            counter.as_ref(),
            MemoryContext {
                warnings,
                operation,
                ..MemoryContext::default()
            },
        )?;
        if partial {
            result.status = "partial".into();
        }
        Ok(result)
    }
    pub async fn valid_references(&self, references: Vec<String>) -> Result<Vec<String>> {
        let mut valid = Vec::new();
        for path in [&self.project_store, &self.user_store] {
            if path.exists() {
                let value = self
                    .client
                    .request(
                        path.clone(),
                        self.context.clone(),
                        Action::ValidateReferences(references.clone()),
                    )
                    .await?;
                valid.extend(serde_json::from_value::<Vec<String>>(value)?);
            }
        }
        valid.sort();
        valid.dedup();
        Ok(valid)
    }
    pub async fn update(&self, request: UpdateRequest) -> Result<serde_json::Value> {
        let scope = request.scope.unwrap_or(Scope::Project);
        let path = self.target(scope).await?;
        self.client
            .request(path, self.context.clone(), Action::Update(request))
            .await
    }
    pub async fn delete(&self, request: DeleteRequest) -> Result<serde_json::Value> {
        let scope = request.scope.unwrap_or(Scope::Project);
        let path = self.target(scope).await?;
        self.client
            .request(path, self.context.clone(), Action::Delete(request))
            .await
    }
    pub async fn operation(&self, id: &str, retry: bool) -> Result<serde_json::Value> {
        self.operation_in_scope(id, retry, Scope::Project).await
    }
    pub async fn operation_in_scope(
        &self,
        id: &str,
        retry: bool,
        scope: Scope,
    ) -> Result<serde_json::Value> {
        let action = if retry {
            Action::Retry(id.into())
        } else {
            Action::Status(id.into())
        };
        self.client
            .request(self.target(scope).await?, self.context.clone(), action)
            .await
    }
    async fn find_anchors(&self, text: &str) -> Result<Vec<Anchor>> {
        let graph = self.graph.as_ref().context("Code grounding unavailable")?;
        let vector: Vec<f32> = serde_json::from_value(
            self.client
                .request(
                    self.project_store.clone(),
                    self.context.clone(),
                    Action::EmbedQuery(text.into()),
                )
                .await?,
        )?;
        let nodes = graph
            .semantic_search_with_context(text, &vector, vector.len(), 3, 0.75, true)
            .await?;
        let ids: Vec<String> = nodes
            .iter()
            .filter_map(|node| node["node_id"].as_str().or_else(|| node["id"].as_str()))
            .map(str::to_string)
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut context = graph.memory_code_context(&ids).await?;
        ensure!(
            context["input_fingerprint"].is_string() && context["graph_complete"] != false,
            "Code index is not ready for memory grounding"
        );
        if let Some(nodes) = context["nodes"].as_array_mut() {
            nodes.retain(|node| {
                node["id"].as_str().is_some_and(|id| {
                    ids.iter().any(|candidate| {
                        candidate.trim_start_matches("nodes:") == id.trim_start_matches("nodes:")
                    })
                })
            });
        }
        self.anchors(&context, false)
    }
    fn anchors(&self, context: &serde_json::Value, supporting: bool) -> Result<Vec<Anchor>> {
        let Some(graph) = &self.graph else {
            return Ok(Vec::new());
        };
        let mut anchors = Vec::new();
        for node in context["nodes"].as_array().into_iter().flatten().take(32) {
            let path = node["file_path"]
                .as_str()
                .or_else(|| node["location"]["file_path"].as_str())
                .unwrap_or("");
            let candidate = self.root.join(path);
            let Ok(canonical) = candidate.canonicalize() else {
                continue;
            };
            if !canonical.starts_with(&self.root) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(canonical) else {
                continue;
            };
            if context["files"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|file| file["file_path"] == path)
                .and_then(|file| file["content_hash"].as_str())
                .is_some_and(|hash| hash != content_hash(&text))
            {
                continue;
            }
            let start = node["start_line"]
                .as_u64()
                .or_else(|| node["location"]["line"].as_u64())
                .unwrap_or(1) as u32;
            let end = node["end_line"]
                .as_u64()
                .or_else(|| node["location"]["end_line"].as_u64())
                .unwrap_or(start as u64) as u32;
            let span = text
                .lines()
                .skip(start.saturating_sub(1) as usize)
                .take((end.saturating_sub(start) + 1) as usize)
                .collect::<Vec<_>>()
                .join("\n");
            let snippet = span.lines().take(40).collect::<Vec<_>>().join("\n");
            let node_id = node["id"]
                .as_str()
                .or_else(|| node["node_id"].as_str())
                .unwrap_or("")
                .to_string();
            if node_id.is_empty() {
                continue;
            }
            anchors.push(Anchor {
                node_id,
                graph_project_id: graph.project_id().into(),
                path: path.into(),
                symbol: node["name"].as_str().unwrap_or("").into(),
                start_line: start,
                end_line: end,
                file_hash: content_hash(&text),
                span_hash: content_hash(&span),
                input_fingerprint: context["input_fingerprint"].as_str().map(str::to_string),
                historical_snippet: snippet,
                supporting,
            });
        }
        Ok(anchors)
    }
    fn enrich(&self, entry: &mut MemoryEntry, context: &serde_json::Value) -> Result<()> {
        for file in context["files"].as_array().into_iter().flatten() {
            if let (Some(path), Some(expected)) =
                (file["file_path"].as_str(), file["content_hash"].as_str())
            {
                let content = self
                    .root
                    .join(path)
                    .canonicalize()
                    .ok()
                    .filter(|v| v.starts_with(&self.root))
                    .and_then(|v| std::fs::read_to_string(v).ok());
                if content.is_none_or(|v| content_hash(&v) != expected) {
                    entry.memory.grounding = Grounding::Unavailable;
                    if entry
                        .memory
                        .anchors
                        .iter()
                        .any(|anchor| anchor.supporting && anchor.path == path)
                    {
                        entry
                            .reasons
                            .push("supporting_evidence_requires_reindex".into());
                    }
                    return Ok(());
                }
            }
        }
        let current = self.anchors(context, false)?;
        for old in &entry.memory.anchors {
            if self
                .graph
                .as_ref()
                .is_none_or(|graph| old.graph_project_id != graph.project_id())
            {
                continue;
            }
            if let Some(current) = current.iter().find(|v| v.node_id == old.node_id) {
                if old.supporting && old.span_hash != current.span_hash {
                    entry.reasons.push("supporting_evidence_changed".into());
                }
            } else if old.supporting {
                entry.reasons.push("supporting_evidence_missing".into());
            }
        }
        // The project input fingerprint is a readiness/cache key, not proof that unrelated
        // code changes affect this claim. Verify explicitly supplied supporting inputs instead.
        for evidence in &entry.memory.evidence {
            if let Some(path) = evidence.uri.strip_prefix("file:")
                && let Some(expected) = &evidence.fingerprint
            {
                let path = self.root.join(path);
                match path
                    .canonicalize()
                    .ok()
                    .filter(|v| v.starts_with(&self.root))
                    .and_then(|v| std::fs::read_to_string(v).ok())
                {
                    Some(content) => {
                        if content_hash(&content) != expected.trim_start_matches("sha256:") {
                            entry.reasons.push("supporting_evidence_changed".into());
                        }
                    }
                    None => entry.reasons.push("supporting_evidence_missing".into()),
                }
            }
        }
        entry.code_context.nodes = context["nodes"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|mut v| {
                if let Some(o) = v.as_object_mut() {
                    o.remove("content");
                    o.remove("metadata");
                    o.insert("snapshot".into(), serde_json::json!("current_index"));
                }
                v
            })
            .collect();
        entry.code_context.edges = context["edges"].as_array().cloned().unwrap_or_default();
        for edge in &entry.code_context.edges {
            entry.code_context.paths.push(serde_json::json!({"kind":"retrieval","edge_id":edge["id"],"from":edge["from"],"to":edge["to"],"code_hops":1,"provenance":edge["metadata"]}));
        }
        for anchor in current {
            entry.code_context.snippets.push(serde_json::json!({"node_id":anchor.node_id,"text":anchor.historical_snippet,"snapshot":"current_source","span_hash":anchor.span_hash,"start_line":anchor.start_line,"end_line":anchor.end_line}));
        }
        entry.reasons.sort();
        entry.reasons.dedup();
        Ok(())
    }
}
fn nonempty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

#[cfg(feature = "ai-enhanced")]
pub struct WorkflowMemory {
    pub runtime: MemoryRuntime,
    pub initial: MemoryContext,
    pub query: String,
    pub collected: tokio::sync::Mutex<Vec<MemoryContext>>,
    pub requests: std::sync::atomic::AtomicUsize,
    pub remaining: tokio::sync::Mutex<std::time::Duration>,
}
#[cfg(feature = "ai-enhanced")]
#[async_trait]
impl codegraph_mcp_rig::memory::MemoryDiscovery for WorkflowMemory {
    fn baseline(&self) -> MemoryContext {
        self.initial.clone()
    }
    async fn discover(&self, result: &serde_json::Value) -> Result<MemoryContext> {
        use std::sync::atomic::Ordering;
        let mut ids = Vec::new();
        let mut symbols = Vec::new();
        collect_discovery(&result["result"], &mut ids, &mut symbols);
        if ids.is_empty() && symbols.is_empty() {
            return Ok(MemoryContext::disabled());
        }
        // Serialize discovery calls across branches and charge provider time only. Model
        // reasoning between tools must not consume the memory foreground allowance.
        let mut remaining = self.remaining.lock().await;
        if self.requests.fetch_add(1, Ordering::SeqCst) >= 8 || remaining.is_zero() {
            return Ok(MemoryContext::unavailable(
                "Workflow memory discovery budget exhausted",
            ));
        }
        ids.sort();
        ids.dedup();
        symbols.sort();
        symbols.dedup();
        symbols.truncate(8);
        let mut request = ReadRequest::new(format!(
            "{} Relevant discovered symbols: {}",
            self.query,
            symbols.join(", ")
        ));
        request.node_ids = ids;
        request.token_budget = self.runtime.settings.config.memory.token_budget;
        request.limit = self.runtime.settings.config.memory.limit;
        let started = tokio::time::Instant::now();
        let result = tokio::time::timeout(*remaining, self.runtime.read(request)).await;
        *remaining = remaining.saturating_sub(started.elapsed());
        let context = result.context("Workflow memory deadline exceeded")??;
        self.collected.lock().await.push(context.clone());
        Ok(context)
    }
}
#[cfg(feature = "ai-enhanced")]
fn collect_discovery(value: &serde_json::Value, ids: &mut Vec<String>, symbols: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                if matches!(key.as_str(), "node_id" | "id")
                    && let Some(id) = value.as_str()
                    && (id.starts_with("nodes:")
                        || uuid::Uuid::parse_str(id).is_ok()
                        || id.len() == 64)
                {
                    ids.push(id.into());
                }
                if matches!(key.as_str(), "name" | "symbol")
                    && let Some(symbol) = value.as_str()
                {
                    symbols.push(symbol.into());
                }
                collect_discovery(value, ids, symbols);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values.iter().take(100) {
                collect_discovery(value, ids, symbols);
            }
        }
        _ => {}
    }
}
