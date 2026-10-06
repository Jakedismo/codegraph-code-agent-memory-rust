// ABOUTME: End-to-end memory owner/CLI tests against loopback mock providers.
// ABOUTME: No user stores, live services, or working code indexes are accessed.
#[cfg(all(
    feature = "memory",
    feature = "ai-enhanced",
    feature = "server-http",
    feature = "embeddings-ollama"
))]
mod enabled {
    use serde_json::{Value, json};
    use std::{
        io::Write,
        path::Path,
        process::{Command, Output, Stdio},
        sync::Arc,
    };
    fn invoke(root: &Path, home: &Path, url: &str, args: &[&str], input: Option<Value>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_codegraph"));
        command
            .args(args)
            .current_dir(root)
            .env("CODEGRAPH_CONFIG_PATH", root.join("memory.toml"))
            .env("CODEGRAPH_MEMORY_HOME", home)
            .env("CODEGRAPH_DEBUG", "0")
            .env("CODEGRAPH_AGENT_ARCHITECTURE", "rig")
            .env("CODEGRAPH_LLM_PROVIDER", "ollama")
            .env("CODEGRAPH_LLM_MODEL", "mock-memory-llm")
            .env("CODEGRAPH_EMBEDDING_PROVIDER", "ollama")
            .env("CODEGRAPH_EMBEDDING_MODEL", "mock-memory-embedding")
            .env("CODEGRAPH_EMBEDDING_DIMENSION", "3")
            .env("CODEGRAPH_OLLAMA_URL", url)
            .env("OLLAMA_API_BASE_URL", url)
            .env(
                "CODEGRAPH_TOKENIZER_PATH",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../codegraph-vector/tokenizers/qwen2.5-coder.json"
                ),
            )
            .env("CODEGRAPH_RERANK_PROVIDER", "none")
            .env("CODEGRAPH_MEMORY_ENABLED", "false")
            .env("CODEGRAPH_USE_GRAPH_SCHEMA", "false")
            .env_remove("CODEGRAPH_MEMORY_TOKEN_BUDGET")
            .env_remove("CODEGRAPH_MEMORY_LIMIT")
            .env_remove("CODEGRAPH_SURREALDB_URL")
            .env_remove("CODEGRAPH_PROJECT_ID")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if args.first() == Some(&"agent") {
            // Agent fixtures select enablement through their own .env or explicit CLI flag.
            command.env_remove("CODEGRAPH_MEMORY_ENABLED");
        }
        let mut child = command.spawn().unwrap();
        if let Some(input) = input {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.to_string().as_bytes())
                .unwrap();
        } else {
            drop(child.stdin.take());
        }
        child.wait_with_output().unwrap()
    }
    fn value(output: Output) -> Value {
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    async fn code_fixture(root: &Path) {
        let project_id = root.canonicalize().unwrap().to_string_lossy().into_owned();
        let source = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
        let mut config = codegraph_graph::SurrealDbConfig::embedded(&root);
        config.database = "codegraph".into();
        let graph = codegraph_graph::SurrealDbStorage::new(config)
            .await
            .unwrap();
        graph.db().query("CREATE nodes:memory_root CONTENT {project_id:$project,name:'score',node_type:'function',file_path:'src/lib.rs',start_line:1,end_line:1}; CREATE nodes:memory_neighbor CONTENT {project_id:$project,name:'helper',node_type:'function',file_path:'src/lib.rs',start_line:2,end_line:2}; CREATE edges:memory_local CONTENT {project_id:$project,from:nodes:memory_root,to:nodes:memory_neighbor,edge_type:'calls',metadata:{resolution:'exact'}}; CREATE project_metadata:memory_ready CONTENT {project_id:$project,name:'fixture',root_path:$project,metadata:{input_fingerprint:'ready-v1',stats:{graph_complete:true}}}; CREATE file_metadata:memory_file CONTENT {project_id:$project,file_path:'src/lib.rs',content_hash:$hash,file_size:36,modified_at:time::now()}; DEFINE FUNCTION OVERWRITE fn::semantic_search_nodes_via_chunks($project: string, $query: string, $dimension: int, $limit: int, $threshold: float, $vector: array<float>) { RETURN (SELECT <string>id AS node_id FROM nodes WHERE project_id=$project AND name='score'); };")
        .bind(("project",project_id.clone())).bind(("hash",codegraph_memory::store::content_hash(&source))).await.unwrap().check().unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn owner_completes_after_writer_exits_and_reopens_without_losing_memory() {
        if let Some(root) = std::env::var_os("CODEGRAPH_MEMORY_CODE_FIXTURE_ROOT") {
            code_fixture(&std::path::PathBuf::from(root)).await;
            return;
        }
        use axum::{
            Json, Router,
            routing::{get, post},
        };
        let app=Router::new()
            .route("/api/tags",get(||async{Json(json!({"models":[{"name":"mock-memory-embedding"}]}))}))
            .route("/api/show",post(||async{Json(json!({"model_info":{"mock.context_length":512},"capabilities":["embedding"]}))}))
            .route("/api/embed",post(|Json(request):Json<Value>|async move{
                let length=request["input"].as_array().map_or(1,Vec::len);
                Json(json!({"model":"mock-memory-embedding","embeddings":vec![vec![1.0,0.0,0.0];length]}))
            }))
            .route("/api/chat",post(|Json(request):Json<Value>|async move{
                let messages = request["messages"].as_array().unwrap();
                let classify = messages.iter().any(|message| message["role"] == "system" && message["content"].as_str().unwrap_or_default().contains("Extract and reconcile agent memories"));
                let content = if classify {
                    json!([{"statement":"Keep scalar scoring stable","kind":"decision","relationships":[],"evidence_indices":[]}]).to_string()
                } else {
                    let context: Option<Value> = messages.iter().filter_map(|message| message["content"].as_str()).find_map(|text|
                        text.split_once("reference:\n").and_then(|(_, json)| serde_json::from_str(json).ok()));
                    if let Some(context) = context {
                        assert_eq!(context["memories"][0]["memory"]["statement"], "Keep scalar scoring stable");
                        format!("Remembered scoring decision [{}]",context["retrieved_memory_refs"][0].as_str().unwrap())
                    } else {
                        "Memory explicitly disabled for this workflow".into()
                    }
                };
                Json(json!({"model":"mock-memory-llm","created_at":"2026-01-01T00:00:00Z","message":{"role":"assistant","content":content},"done":true}))
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let temporary = Arc::new(tempfile::tempdir().unwrap());
        let root = temporary.path().join("project");
        let home = temporary.path().join("home");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".env"), "").unwrap();
        std::fs::write(root.join("memory.toml"),"[embedding]\nprovider = 'ollama'\nmodel = 'mock-memory-embedding'\ndimension = 3\n[rerank]\nprovider = 'none'\n").unwrap();
        let task = tokio::task::spawn_blocking(move || {
            let accepted = value(invoke(
                &root,
                &home,
                &url,
                &[
                    "memory",
                    "write",
                    "Keep scalar scoring stable",
                    "--idempotency-key",
                    "request-1",
                ],
                None,
            ));
            assert_eq!(accepted["embedding_ready"], true);
            let id = accepted["id"].as_str().unwrap();
            let completed = value(invoke(
                &root,
                &home,
                &url,
                &["memory", "wait", id, "--timeout-secs", "20"],
                None,
            ));
            assert_eq!(completed["state"], "complete");
            let recalled = value(invoke(
                &root,
                &home,
                &url,
                &[
                    "memory",
                    "read",
                    "scoring",
                    "--scope",
                    "both",
                    "--limit",
                    "1",
                    "--token-budget",
                    "3000",
                ],
                None,
            ));
            assert_eq!(
                recalled["memories"][0]["memory"]["statement"],
                "Keep scalar scoring stable"
            );
            assert!(!root.join(".codegraph/db").exists());
            for workflow in ["context", "impact", "architecture", "quality"] {
                let result = value(invoke(
                    &root,
                    &home,
                    &url,
                    &[
                        "agent",
                        workflow,
                        "Explain scoring",
                        "--memory",
                        "on",
                        "--timeout-secs",
                        "20",
                    ],
                    None,
                ));
                assert_eq!(
                    result["memory_context"]["memories"][0]["memory"]["statement"],
                    "Keep scalar scoring stable",
                    "{result}"
                );
                assert_eq!(
                    result["memory_context"]["cited_memory_refs"]
                        .as_array()
                        .unwrap()
                        .len(),
                    1
                );
                assert_eq!(
                    result["memory_context"]["cited_memory_refs"],
                    result["memory_context"]["retrieved_memory_refs"]
                );
            }
            // Automatic mode must load all three memory settings from the project .env.
            std::fs::write(root.join(".env"), "CODEGRAPH_MEMORY_ENABLED=true\nCODEGRAPH_MEMORY_TOKEN_BUDGET=6000\nCODEGRAPH_MEMORY_LIMIT=3\n").unwrap();
            let automatic = value(invoke(
                &root,
                &home,
                &url,
                &[
                    "agent",
                    "context",
                    "Explain scoring",
                    "--timeout-secs",
                    "20",
                ],
                None,
            ));
            assert_eq!(
                automatic["memory_context"]["memories"][0]["memory"]["statement"],
                "Keep scalar scoring stable"
            );
            assert_eq!(automatic["memory_context"]["token_budget"], 6000);
            assert_eq!(automatic["memory_context"]["limit"], 3);
            let disabled = value(invoke(
                &root,
                &home,
                &url,
                &[
                    "agent",
                    "context",
                    "Explain scoring",
                    "--memory",
                    "off",
                    "--timeout-secs",
                    "20",
                ],
                None,
            ));
            assert_eq!(disabled["memory_context"]["status"], "disabled");
            assert!(
                disabled["memory_context"]["retrieved_memory_refs"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            std::fs::write(root.join(".env"), "").unwrap();
            value(invoke(
                &root,
                &home,
                &url,
                &["memory", "service", "stop"],
                None,
            ));
            std::thread::sleep(std::time::Duration::from_millis(250));
            let recalled = value(invoke(
                &root,
                &home,
                &url,
                &["memory", "read", "scoring"],
                None,
            ));
            assert_eq!(
                recalled["memories"][0]["memory"]["statement"],
                "Keep scalar scoring stable"
            );
            // Add a current code fixture after the owner restart. The semantic function
            // deliberately returns the production `node_id` shape, rather than `id`.
            std::fs::create_dir_all(root.join("src")).unwrap();
            let source = "fn score() { 1.0 }\nfn helper() { 2.0 }\n";
            std::fs::write(root.join("src/lib.rs"), source).unwrap();
            let project_id = root.canonicalize().unwrap().to_string_lossy().into_owned();
            let fixture = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "enabled::owner_completes_after_writer_exits_and_reopens_without_losing_memory",
                    "--test-threads=1",
                ])
                .env("CODEGRAPH_MEMORY_CODE_FIXTURE_ROOT", &root)
                .output()
                .unwrap();
            assert!(
                fixture.status.success(),
                "{}",
                String::from_utf8_lossy(&fixture.stderr)
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
            let anchor = codegraph_memory::Anchor {
                node_id: "nodes:memory_root".into(),
                graph_project_id: project_id,
                supporting: true,
                ..Default::default()
            };
            let accepted = value(invoke(
                &root,
                &home,
                &url,
                &["memory", "write", "--input", "-"],
                Some(
                    json!({"statement":"Keep scalar scoring stable","code_related":true,"anchors":[anchor]}),
                ),
            ));
            value(invoke(
                &root,
                &home,
                &url,
                &["memory", "wait", accepted["id"].as_str().unwrap()],
                None,
            ));
            let request = json!({"query":"scoring","token_budget":20000});
            let grounded = value(invoke(
                &root,
                &home,
                &url,
                &["memory", "read", "--input", "-"],
                Some(request.clone()),
            ));
            let entry = grounded["memories"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["memory"]["grounding"] == "anchored")
                .unwrap_or_else(|| panic!("Missing anchored memory: {grounded}"));
            assert_eq!(entry["memory"]["grounding"], "anchored", "{grounded}");
            assert!(
                entry["code_context"]["paths"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|path| path["memory_anchor"].is_string()
                        && path["code_hop_count"] == 0
                        && path["anchor_role"] == "evidence"),
                "Native projection join must reach the returned memory: {grounded}"
            );
            assert!(
                entry["code_context"]["paths"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|path| path["code_hops"] == 1)
            );
            assert!(
                entry["code_context"]["snippets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|snippet| snippet["snapshot"] == "historical")
            );
            assert!(
                entry["code_context"]["snippets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|snippet| snippet["snapshot"] == "current_source")
            );
            std::fs::write(
                root.join("src/lib.rs"),
                "fn score() { 99.0 }\nfn helper() { 2.0 }\n",
            )
            .unwrap();
            let changed = value(invoke(
                &root,
                &home,
                &url,
                &["memory", "read", "--input", "-"],
                Some(request),
            ));
            assert!(
                changed["memories"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|entry| entry["memory"]["grounding"] != "anchored"),
                "{changed}"
            );
            assert!(
                changed["needs_verification"][0]["reasons"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|reason| reason == "supporting_evidence_requires_reindex")
            );
            let selection = value(invoke(
                &root,
                &home,
                &url,
                &["memory", "delete", "--query", "scoring"],
                None,
            ));
            assert_eq!(selection["status"], "needs_selection");
            let claim = &recalled["memories"][0]["memory"];
            let claim_id = claim["id"].as_str().unwrap();
            let revision = claim["revision"].as_u64().unwrap().to_string();
            let corrected = value(invoke(
                &root,
                &home,
                &url,
                &[
                    "memory",
                    "update",
                    "Keep scalar scoring stable",
                    "--memory-id",
                    claim_id,
                    "--expected-revision",
                    &revision,
                ],
                None,
            ));
            assert!(corrected["revision"].as_u64().unwrap() > claim["revision"].as_u64().unwrap());
            value(invoke(
                &root,
                &home,
                &url,
                &[
                    "memory",
                    "wait",
                    corrected["operation_id"].as_str().unwrap(),
                ],
                None,
            ));
            let stale_delete = invoke(
                &root,
                &home,
                &url,
                &[
                    "memory",
                    "delete",
                    "--memory-id",
                    claim_id,
                    "--expected-revision",
                    &revision,
                ],
                None,
            );
            assert!(!stale_delete.status.success());
            assert!(String::from_utf8_lossy(&stale_delete.stdout).contains("Revision conflict"));
            let target = &changed["needs_verification"][0]["memory"];
            let target_revision = target["revision"].as_u64().unwrap().to_string();
            let deleted = value(invoke(
                &root,
                &home,
                &url,
                &[
                    "memory",
                    "delete",
                    "--memory-id",
                    target["id"].as_str().unwrap(),
                    "--expected-revision",
                    &target_revision,
                ],
                None,
            ));
            assert_eq!(deleted["status"], "forgotten");
            std::fs::write(
                root.join(".env"),
                "CODEGRAPH_EMBEDDING_QUERY_PREFIX=changed-prefix: \n",
            )
            .unwrap();
            let mismatch = invoke(&root, &home, &url, &["memory", "read", "scoring"], None);
            assert!(!mismatch.status.success());
            assert!(String::from_utf8_lossy(&mismatch.stdout).contains("runtime controls differ"));
            value(invoke(
                &root,
                &home,
                &url,
                &["memory", "service", "stop"],
                None,
            ));
            drop(temporary);
        });
        task.await.unwrap();
        server.abort();
    }
}

#[test]
fn memory_help_and_init_guidance_require_no_providers() {
    let project = tempfile::tempdir().unwrap();
    let help = std::process::Command::new(env!("CARGO_BIN_EXE_codegraph"))
        .args(["memory", "--help"])
        .current_dir(project.path())
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    for command in [
        "write", "read", "update", "delete", "status", "wait", "retry", "reembed", "service",
    ] {
        assert!(help.contains(command));
    }
    for (command, flags) in [
        (
            "write",
            vec!["--idempotency-key", "--asynchronous", "--ttl-seconds"],
        ),
        (
            "read",
            vec!["--limit", "--token-budget", "--include-provisional"],
        ),
        (
            "update",
            vec!["--memory-id", "--expected-revision", "--complete"],
        ),
        (
            "delete",
            vec!["--memory-id", "--expected-revision", "--query"],
        ),
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_codegraph"))
            .args(["memory", command, "--help"])
            .current_dir(project.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        let help = String::from_utf8(output.stdout).unwrap();
        for flag in flags {
            assert!(help.contains(flag), "{help}");
        }
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_codegraph"))
        .args(["init", "--hooks", "none", "--no-index"])
        .current_dir(project.path())
        .env("CODEGRAPH_EMBEDDING_PROVIDER", "invalid-do-not-load")
        .output()
        .unwrap();
    assert!(output.status.success());
    for name in ["AGENTS.md", "CLAUDE.md"] {
        let text = std::fs::read_to_string(project.path().join(name)).unwrap();
        assert!(text.contains("memory_write"));
        assert!(text.contains("needs_verification"));
        assert!(text.contains("--scope session"));
    }
    assert!(!project.path().join(".codegraph/memory-db").exists());
    assert!(
        !project
            .path()
            .join(".codegraph/memory-project.json")
            .exists()
    );
}
