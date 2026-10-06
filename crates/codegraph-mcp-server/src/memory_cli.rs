// ABOUTME: CLI equivalents of the four semantic memory tools and operational job controls.
// ABOUTME: Provider/environment preparation precedes runtime startup; service commands do not initialize code indexes.
#[cfg(feature = "memory")]
use anyhow::Context;
use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Debug, Args)]
pub struct MemoryArgs {
    #[arg(long, default_value = ".")]
    pub project: PathBuf,
    #[arg(long)]
    pub session_id: Option<String>,
    #[arg(long)]
    pub context_epoch: Option<String>,
    /// UTF-8 JSON request; use '-' for stdin.
    #[arg(long)]
    pub input: Option<PathBuf>,
    #[arg(long,default_value_t=120,value_parser=clap::value_parser!(u64).range(1..))]
    pub timeout_secs: u64,
    /// Read scope defaults to both; writes default to project. Session requires --session-id.
    #[arg(long,value_parser=["both","project","session","user"])]
    pub scope: Option<String>,
}
#[derive(Debug, Args, serde::Serialize)]
pub struct WriteOptions {
    /// Deduplicate retries of this observation within its access scope.
    #[arg(long)]
    pub idempotency_key: Option<String>,
    /// Explicit review/expiry horizon in seconds.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub ttl_seconds: Option<u64>,
    /// Accept durably before embedding; pending observations cannot be recalled yet.
    #[arg(long)]
    pub asynchronous: bool,
    /// Request code grounding; missing supporting evidence is reported explicitly.
    #[arg(long)]
    pub code_related: bool,
}
#[derive(Debug, Args, serde::Serialize)]
pub struct ReadOptions {
    /// Maximum number of selected claims.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub limit: Option<u64>,
    /// Budget for the complete memory context, including wrappers and citations.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub token_budget: Option<u64>,
    #[arg(long)]
    pub symbol: Option<String>,
    #[arg(long)]
    pub node_id: Option<String>,
    /// Include unprocessed observations with compatible embeddings.
    #[arg(long)]
    pub include_provisional: bool,
    #[arg(long)]
    pub include_archived: bool,
    #[arg(long)]
    pub include_expired: bool,
    #[arg(long)]
    pub include_stale: bool,
}
#[derive(Debug, Args, serde::Serialize)]
pub struct MemorySelector {
    /// Exact claim identity from memory read; requires its current revision.
    #[arg(long, conflicts_with = "query")]
    pub memory_id: Option<String>,
    /// Revision checked atomically before changing or forgetting the claim.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..), conflicts_with = "query")]
    pub expected_revision: Option<u64>,
    /// Find candidate claims for selection; this query does not mutate them.
    #[arg(long)]
    pub query: Option<String>,
}
#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    /// Submit an observation for semantic acceptance and background classification.
    Write {
        /// Free-text observation; alternatively supply a JSON request with --input.
        statement: Option<String>,
        #[command(flatten)]
        options: WriteOptions,
        #[command(flatten)]
        args: MemoryArgs,
    },
    /// Recall semantic memories with code context and verification warnings.
    Read {
        query: Option<String>,
        #[command(flatten)]
        options: ReadOptions,
        #[command(flatten)]
        args: MemoryArgs,
    },
    /// Correct a selected revision, complete working state, or select candidates.
    Update {
        /// Replacement observation; evidence/confirmation/feedback use --input.
        statement: Option<String>,
        #[command(flatten)]
        selector: MemorySelector,
        /// Archive completed working/session state.
        #[arg(long)]
        complete: bool,
        #[command(flatten)]
        args: MemoryArgs,
    },
    /// Forget a selected revision and its derivatives, or select candidates.
    Delete {
        #[command(flatten)]
        selector: MemorySelector,
        #[command(flatten)]
        args: MemoryArgs,
    },
    /// Inspect a durable background operation.
    Status {
        operation_id: String,
        #[command(flatten)]
        args: MemoryArgs,
    },
    /// Wait for completion, failure, cancellation, or required configuration.
    Wait {
        operation_id: String,
        #[command(flatten)]
        args: MemoryArgs,
    },
    /// Retry a failed/config-required background operation.
    Retry {
        operation_id: String,
        #[command(flatten)]
        args: MemoryArgs,
    },
    /// Explicitly migrate all current and historical embeddings to the configured provider identity.
    Reembed {
        #[command(flatten)]
        args: MemoryArgs,
    },
    /// Inspect or stop the shared local memory owner.
    Service {
        #[command(subcommand)]
        action: ServiceCommand,
    },
}
#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    #[command(hide = true)]
    Run {
        #[arg(long)]
        runtime_dir: PathBuf,
    },
    Status,
    Stop,
}
#[cfg(any(feature = "memory", test))]
impl MemoryCommand {
    /// Explicit flags override corresponding JSON fields; absent flags preserve them.
    fn request_input(&self, mut input: serde_json::Value) -> Result<serde_json::Value> {
        let object = input
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Memory --input must contain a JSON object"))?;
        let merge = |object: &mut serde_json::Map<String, serde_json::Value>, options| {
            if let serde_json::Value::Object(fields) = options {
                object.extend(fields.into_iter().filter(|(_, value)| {
                    !value.is_null() && value != &serde_json::Value::Bool(false)
                }));
            }
        };
        let args = match self {
            Self::Write {
                statement,
                options,
                args,
            } => {
                merge(object, serde_json::to_value(options)?);
                if let Some(statement) = statement {
                    object.insert("statement".into(), statement.clone().into());
                }
                args
            }
            Self::Read {
                query,
                options,
                args,
            } => {
                merge(object, serde_json::to_value(options)?);
                if let Some(query) = query {
                    object.insert("query".into(), query.clone().into());
                }
                args
            }
            Self::Update {
                statement,
                selector,
                complete,
                args,
            } => {
                merge(object, serde_json::to_value(selector)?);
                if let Some(statement) = statement {
                    object.insert("statement".into(), statement.clone().into());
                }
                if *complete {
                    object.insert("complete".into(), true.into());
                }
                args
            }
            Self::Delete { selector, args } => {
                merge(object, serde_json::to_value(selector)?);
                args
            }
            Self::Status { args, .. }
            | Self::Wait { args, .. }
            | Self::Retry { args, .. }
            | Self::Reembed { args } => args,
            Self::Service { .. } => return Ok(input),
        };
        if let Some(scope) = &args.scope {
            object.insert("scope".into(), scope.clone().into());
        }
        if !matches!(self, Self::Read { .. })
            && object.get("scope").is_some_and(|scope| scope == "both")
        {
            anyhow::bail!(
                "Scope 'both' is only valid for memory read; select one scope for other operations"
            );
        }
        if matches!(self, Self::Update { .. } | Self::Delete { .. }) {
            let has_id = object
                .get("memory_id")
                .is_some_and(|value| !value.is_null());
            let has_revision = object
                .get("expected_revision")
                .is_some_and(|value| !value.is_null());
            let has_query = object.get("query").is_some_and(|value| !value.is_null());
            anyhow::ensure!(
                has_id == has_revision,
                "Select both memory_id and expected_revision for mutation"
            );
            anyhow::ensure!(
                has_id != has_query,
                "Supply either a semantic query for selection or memory_id with expected_revision"
            );
        }
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use serde_json::json;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        command: MemoryCommand,
    }
    fn request(args: &[&str], input: serde_json::Value) -> Result<serde_json::Value> {
        Cli::try_parse_from(std::iter::once("memory").chain(args.iter().copied()))?
            .command
            .request_input(input)
    }

    #[test]
    fn memory_flags_preserve_evidence_and_override_explicit_fields() {
        let input = json!({"statement":"old", "evidence":[{"uri":"test:regression"}], "asynchronous":true, "ttl_seconds":120});
        let written = request(
            &[
                "write",
                "new",
                "--idempotency-key",
                "retry-1",
                "--scope",
                "session",
                "--session-id",
                "task-1",
            ],
            input.clone(),
        )
        .unwrap();
        assert_eq!(written["statement"], "new");
        assert_eq!(written["evidence"], input["evidence"]);
        assert_eq!(written["asynchronous"], true);
        assert_eq!(written["ttl_seconds"], 120);
        assert_eq!(written["scope"], "session");
        assert_eq!(written["idempotency_key"], "retry-1");
        let read = request(
            &[
                "read",
                "constraints",
                "--scope",
                "both",
                "--limit",
                "3",
                "--token-budget",
                "900",
                "--include-archived",
            ],
            json!({}),
        )
        .unwrap();
        assert_eq!(read["query"], "constraints");
        assert_eq!(read["limit"], 3);
        assert_eq!(read["token_budget"], 900);
        assert_eq!(read["include_archived"], true);
        assert!(read.get("include_stale").is_none());
    }

    #[test]
    fn mutation_flags_require_explicit_selection_and_revision() {
        let corrected = request(
            &[
                "update",
                "new",
                "--memory-id",
                "claim-1",
                "--expected-revision",
                "2",
            ],
            json!({}),
        )
        .unwrap();
        assert_eq!(corrected["statement"], "new");
        assert_eq!(corrected["memory_id"], "claim-1");
        assert_eq!(corrected["expected_revision"], 2);
        let selected = request(&["delete", "--query", "scoring"], json!({})).unwrap();
        assert_eq!(selected["query"], "scoring");
        assert!(selected.get("memory_id").is_none());
        assert!(request(&["delete", "--memory-id", "claim-1"], json!({})).is_err());
        assert!(request(&["delete"], json!({})).is_err());
        assert!(
            request(
                &["delete", "--query", "scoring"],
                json!({"memory_id":"claim-1", "expected_revision":2})
            )
            .is_err()
        );
        assert!(request(&["write", "text", "--scope", "both"], json!({})).is_err());
        assert!(request(&["read", "text"], json!([])).is_err());
        assert!(request(&["read", "text", "--limit", "0"], json!({})).is_err());
    }
}
/// # Safety
/// Must run at the single-threaded CLI entry point before application threads start.
pub unsafe fn run(command: &MemoryCommand, config: Option<&Path>) -> Result<()> {
    #[cfg(not(feature = "memory"))]
    {
        let _ = (command, config);
        anyhow::bail!("Memory requires a build with --features memory (or full)");
    }
    #[cfg(feature = "memory")]
    {
        use crate::memory_runtime::{MemoryRuntime, run_owner, runtime_dir};
        use codegraph_memory::{
            owner::{Action, OwnerClient},
            types::*,
        };
        let args = match command {
            MemoryCommand::Write { args, .. }
            | MemoryCommand::Read { args, .. }
            | MemoryCommand::Update { args, .. }
            | MemoryCommand::Delete { args, .. }
            | MemoryCommand::Status { args, .. }
            | MemoryCommand::Wait { args, .. }
            | MemoryCommand::Retry { args, .. }
            | MemoryCommand::Reembed { args } => Some(args),
            _ => None,
        };
        let config = config.map(Path::canonicalize).transpose()?;
        let input_path = args
            .and_then(|v| v.input.as_ref())
            .filter(|v| *v != Path::new("-"))
            .map(|path| path.canonicalize())
            .transpose()?;
        if let Some(args) = args {
            std::env::set_current_dir(&args.project).context("Open memory project")?;
        }
        // SAFETY: Called at the single-threaded entry point.
        unsafe {
            codegraph_core::config_manager::ConfigManager::initialize_environment();
            if let Some(config) = &config {
                std::env::set_var("CODEGRAPH_CONFIG_PATH", config);
            }
            if let Some(args) = args {
                if let Some(id) = &args.session_id {
                    std::env::set_var("CODEGRAPH_MEMORY_SESSION_ID", id);
                }
                if let Some(epoch) = &args.context_epoch {
                    std::env::set_var("CODEGRAPH_MEMORY_CONTEXT_EPOCH", epoch);
                }
            }
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let response=runtime.block_on(async {
            if let MemoryCommand::Service{action}=command {
                return match action {
                    ServiceCommand::Run{runtime_dir}=>{run_owner(runtime_dir).await?;Ok(serde_json::json!({"status":"stopped"}))},
                    ServiceCommand::Status=>Ok(serde_json::json!({"status":if OwnerClient::existing(&runtime_dir()?).await.is_ok(){"running"}else{"stopped"}})),
                    ServiceCommand::Stop=>{let client=OwnerClient::existing(&runtime_dir()?).await?;client.request(PathBuf::new(),ClientContext{owner_id:String::new(),project_id:String::new(),session_id:None,task_id:None,context_epoch:None,applicability:Default::default()},Action::Stop).await},
                };
            }
            let args=args.context("Missing memory arguments")?;
            let input=if let Some(path)=&args.input {
                let text=if path==Path::new("-"){use std::io::Read;let mut text=String::new();std::io::stdin().take(8*1024*1024).read_to_string(&mut text)?;text}else{std::fs::read_to_string(input_path.as_ref().context("Missing input path")?)?};
                serde_json::from_str::<serde_json::Value>(&text)?
            }else{serde_json::json!({})};
            let input=command.request_input(input)?;
            // Reject malformed requests before opening stores or initializing providers.
            match command {
                MemoryCommand::Write{..}=>{let _:WriteRequest=serde_json::from_value(input.clone())?;},
                MemoryCommand::Read{..}=>{let _:ReadRequest=serde_json::from_value(input.clone())?;},
                MemoryCommand::Update{..}=>{let _:UpdateRequest=serde_json::from_value(input.clone())?;},
                MemoryCommand::Delete{..}=>{let _:DeleteRequest=serde_json::from_value(input.clone())?;},
                _=>{},
            }
            let config=codegraph_core::config_manager::ConfigManager::load()?;
            tokio::time::timeout(std::time::Duration::from_secs(args.timeout_secs),async {
                let memory=if matches!(command,MemoryCommand::Reembed{..}){MemoryRuntime::connect_for_reembed(config.config().clone()).await?}else{MemoryRuntime::connect(config.config().clone(),None).await?};
                match command {
                    MemoryCommand::Write{..}=>{let request:WriteRequest=serde_json::from_value(input)?;memory.write(request).await},
                    MemoryCommand::Read{..}=>{let request:ReadRequest=serde_json::from_value(input)?;Ok(serde_json::to_value(memory.read(request).await?)?)},
                    MemoryCommand::Update{..}=>memory.update(serde_json::from_value(input)?).await,
                    MemoryCommand::Delete{..}=>memory.delete(serde_json::from_value(input)?).await,
                    MemoryCommand::Status{operation_id,..}=>memory.operation_in_scope(operation_id,false,serde_json::from_value(input.get("scope").cloned().unwrap_or(serde_json::json!("project")))?).await,
                    MemoryCommand::Retry{operation_id,..}=>memory.operation_in_scope(operation_id,true,serde_json::from_value(input.get("scope").cloned().unwrap_or(serde_json::json!("project")))?).await,
                    MemoryCommand::Reembed{..}=>memory.reembed(serde_json::from_value(input.get("scope").cloned().unwrap_or(serde_json::json!("project")))?).await,
                    MemoryCommand::Wait{operation_id,..}=>loop {
                        let value=memory.operation_in_scope(operation_id,false,serde_json::from_value(input.get("scope").cloned().unwrap_or(serde_json::json!("project")))?).await?;
                        let state=value["state"].as_str().unwrap_or("");
                        if matches!(state,"complete"|"failed"|"cancelled"|"config_required"){break Ok(value);}
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    },
                    _=>anyhow::bail!("Unsupported memory command"),
                }
            }).await.context("Memory command deadline exceeded")?
        });
        match response {
            Ok(response) => {
                println!("{}", serde_json::to_string(&response)?);
                Ok(())
            }
            Err(error) => {
                println!(
                    "{}",
                    serde_json::json!({"error":{"message":format!("{error:#}")}})
                );
                Err(error)
            }
        }
    }
}
