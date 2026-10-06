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
    #[arg(long,value_parser=["project","session","user"])]
    pub scope: Option<String>,
}
#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    Write {
        statement: Option<String>,
        #[command(flatten)]
        args: MemoryArgs,
    },
    Read {
        query: Option<String>,
        #[command(flatten)]
        args: MemoryArgs,
    },
    Update {
        #[command(flatten)]
        args: MemoryArgs,
    },
    Delete {
        #[command(flatten)]
        args: MemoryArgs,
    },
    Status {
        operation_id: String,
        #[command(flatten)]
        args: MemoryArgs,
    },
    Wait {
        operation_id: String,
        #[command(flatten)]
        args: MemoryArgs,
    },
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
            | MemoryCommand::Update { args }
            | MemoryCommand::Delete { args }
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
            let mut input=input;
            if let Some(scope)=&args.scope {input["scope"]=serde_json::json!(scope);}
            let config=codegraph_core::config_manager::ConfigManager::load()?;
            tokio::time::timeout(std::time::Duration::from_secs(args.timeout_secs),async {
                let memory=if matches!(command,MemoryCommand::Reembed{..}){MemoryRuntime::connect_for_reembed(config.config().clone()).await?}else{MemoryRuntime::connect(config.config().clone(),None).await?};
                match command {
                    MemoryCommand::Write{statement,..}=>{if let Some(statement)=statement {input["statement"]=serde_json::json!(statement);}let request:WriteRequest=serde_json::from_value(input)?;memory.write(request).await},
                    MemoryCommand::Read{query,..}=>{if let Some(query)=query{input["query"]=serde_json::json!(query);}let request:ReadRequest=serde_json::from_value(input)?;Ok(serde_json::to_value(memory.read(request).await?)?)},
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
