// ABOUTME: Authenticated local memory owner IPC and durable worker lifetime.
// ABOUTME: Only this process opens memory stores; code indexes remain owned by code clients.
use crate::{MemoryService, types::*};
use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::Mutex,
};
use uuid::Uuid;
const MAX_FRAME: u64 = 8 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub struct Registration {
    pub store_path: PathBuf,
    pub settings: serde_json::Value,
}
#[derive(Serialize, Deserialize)]
pub enum Action {
    Register(Registration),
    Write(WriteRequest),
    Read(ReadRequest),
    Update(UpdateRequest),
    Delete(DeleteRequest),
    Status(String),
    Retry(String),
    Stop,
    EmbedQuery(String),
    GroundingChecks(Vec<GroundingCheck>),
    Reembed(Registration),
    ValidateReferences(Vec<String>),
    ProjectionChanges {
        graph_project_id: String,
        cursor: codegraph_core::memory_projection::ProjectionCursor,
    },
}
#[derive(Serialize, Deserialize)]
pub struct Request {
    pub token: String,
    pub store_path: PathBuf,
    pub context: ClientContext,
    pub action: Action,
}
#[derive(Serialize, Deserialize)]
struct Response {
    value: Option<serde_json::Value>,
    error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Discovery {
    version: u32,
    address: String,
    token: String,
}

#[async_trait]
pub trait ServiceFactory: Send + Sync {
    async fn create(&self, registration: Registration) -> Result<MemoryService>;
    async fn configure(
        &self,
        existing: MemoryService,
        registration: Registration,
    ) -> Result<MemoryService>;
    async fn recover(&self, path: PathBuf) -> Result<MemoryService>;
    async fn reembed(
        &self,
        existing: MemoryService,
        registration: Registration,
    ) -> Result<MemoryService>;
}

#[derive(Clone)]
pub struct OwnerClient {
    discovery: Discovery,
}
impl OwnerClient {
    pub async fn connect(runtime_dir: &Path, binary: &Path) -> Result<Self> {
        private_dir(runtime_dir)?;
        if let Ok(client) = Self::existing(runtime_dir).await {
            return Ok(client);
        }
        // Concurrent starts race safely on an OS lock held for the entire service lifetime.
        std::process::Command::new(binary)
            .args(["memory", "service", "run", "--runtime-dir"])
            .arg(runtime_dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("Start memory owner")?;
        for _ in 0..200 {
            if let Ok(client) = Self::existing(runtime_dir).await {
                return Ok(client);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        bail!("Memory owner did not become available within 10 seconds")
    }
    pub async fn existing(runtime_dir: &Path) -> Result<Self> {
        let path = runtime_dir.join("owner.json");
        private_file(&path)?;
        let discovery: Discovery = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(discovery.version == 1, "Incompatible memory owner protocol");
        let address: std::net::SocketAddr = discovery.address.parse()?;
        ensure!(address.ip().is_loopback(), "Memory owner must use loopback");
        tokio::time::timeout(Duration::from_secs(1), TcpStream::connect(address)).await??;
        Ok(Self { discovery })
    }
    pub async fn request(
        &self,
        store_path: PathBuf,
        context: ClientContext,
        action: Action,
    ) -> Result<serde_json::Value> {
        let request = Request {
            token: self.discovery.token.clone(),
            store_path,
            context,
            action,
        };
        let mut stream = TcpStream::connect(&self.discovery.address).await?;
        let mut bytes = serde_json::to_vec(&request)?;
        ensure!(
            bytes.len() < MAX_FRAME as usize,
            "Memory request is too large"
        );
        bytes.push(b'\n');
        stream.write_all(&bytes).await?;
        let frame = read_frame(stream).await?;
        let response: Response = serde_json::from_slice(&frame)?;
        match (response.value, response.error) {
            (Some(value), None) => Ok(value),
            (_, Some(error)) => bail!("{error}"),
            _ => bail!("Invalid memory owner response"),
        }
    }
}
async fn read_frame(stream: TcpStream) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    let mut reader = BufReader::new(stream.take(MAX_FRAME));
    reader.read_until(b'\n', &mut bytes).await?;
    ensure!(
        bytes.len() < MAX_FRAME as usize && bytes.last() == Some(&b'\n'),
        "Oversized or incomplete IPC frame"
    );
    Ok(bytes)
}

pub async fn run(runtime_dir: &Path, factory: Arc<dyn ServiceFactory>) -> Result<()> {
    private_dir(runtime_dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(runtime_dir.join("owner.lock"))?;
    if lock.try_lock().is_err() {
        return Ok(());
    }
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let discovery = Discovery {
        version: 1,
        address: listener.local_addr()?.to_string(),
        token: Uuid::new_v4().to_string() + &Uuid::new_v4().to_string(),
    };
    let path = runtime_dir.join("owner.json");
    let temporary = runtime_dir.join(format!("owner-{}.tmp", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(&serde_json::to_vec(&discovery)?)?;
    file.sync_all()?;
    std::fs::rename(&temporary, &path)?;
    let services: Arc<Mutex<BTreeMap<PathBuf, MemoryService>>> =
        Arc::new(Mutex::new(BTreeMap::new()));
    let registry_path = runtime_dir.join("stores.json");
    if let Ok(bytes) = std::fs::read(&registry_path)
        && let Ok(paths) = serde_json::from_slice::<Vec<PathBuf>>(&bytes)
    {
        for path in paths {
            if path.exists()
                && let Ok(service) = factory.recover(path.clone()).await
            {
                services.lock().await.insert(path, service);
            }
        }
    }
    let last_activity = Arc::new(Mutex::new(Instant::now()));
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let worker_services = services.clone();
    let worker_activity = last_activity.clone();
    let worker_shutdown = shutdown.clone();
    let worker = tokio::spawn(async move {
        let mut cursor = 0usize;
        loop {
            let all: Vec<_> = worker_services.lock().await.values().cloned().collect();
            let mut active = false;
            // One global classifier call at a time; rotate stores to avoid starvation.
            if !all.is_empty() {
                let length = all.len();
                for index in 0..length {
                    let service = &all[(index + cursor) % length];
                    if service.process_next().await.unwrap_or(false) {
                        active = true;
                    }
                }
                cursor = (cursor + 1) % length;
            }
            if active {
                *worker_activity.lock().await = Instant::now();
            }
            if !active && worker_activity.lock().await.elapsed() > Duration::from_secs(300) {
                worker_shutdown.notify_one();
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
    let clients = Arc::new(tokio::sync::Semaphore::new(16));
    let result:Result<()>=async {
        loop {
            let (stream,_)=tokio::select! {incoming=listener.accept()=>incoming?,_ = shutdown.notified()=>break};
            let Ok(permit) = clients.clone().try_acquire_owned() else { drop(stream); continue; };
            let services=services.clone();let factory=factory.clone();let token=discovery.token.clone();let activity=last_activity.clone();let shutdown=shutdown.clone();let registry_path=registry_path.clone();
            tokio::spawn(async move {
                let _permit = permit;
                // Split here so the bounded reader can consume a cloned read-half without cloning sockets.
                let (read,mut write)=stream.into_split();
                use tokio::io::AsyncReadExt;
                let result:Result<serde_json::Value>=async {
                    let mut frame=Vec::new();let mut reader=BufReader::new(read.take(MAX_FRAME));
                    tokio::time::timeout(Duration::from_secs(10),reader.read_until(b'\n',&mut frame)).await??;
                    ensure!(frame.len()<MAX_FRAME as usize && frame.last()==Some(&b'\n'),"Invalid memory owner frame");
                    let request:Request=serde_json::from_slice(&frame)?;
                    ensure!(request.token==token,"Memory owner authentication failed");
                    *activity.lock().await=Instant::now();
                    if let Action::Stop=request.action {shutdown.notify_one();return Ok(serde_json::json!({"status":"stopped"}));}
                    if let Action::Register(registration)=request.action {
                        ensure!(registration.store_path==request.store_path,"Store registration mismatch");
                        let mut guard=services.lock().await;
                        if let Some(existing)=guard.get(&request.store_path) {
                            let configured=factory.configure(existing.clone(),registration).await?;
                            guard.insert(request.store_path,configured);
                            return Ok(serde_json::json!({"status":"registered","memory_schema_version":2}));
                        }
                        let service=factory.create(registration).await?;
                        guard.insert(request.store_path,service);
                        // Discovery metadata contains paths only, never provider settings or credentials.
                        let registry=guard.keys().cloned().collect::<Vec<_>>();
                        let temporary=registry_path.with_extension("tmp");std::fs::write(&temporary,serde_json::to_vec(&registry)?)?;std::fs::rename(temporary,&registry_path)?;
                        return Ok(serde_json::json!({"status":"registered","memory_schema_version":2}));
                    }
                    let service=services.lock().await.get(&request.store_path).cloned().context("Register memory store configuration first")?;
                    if let Action::Reembed(registration)=request.action {
                        ensure!(registration.store_path==request.store_path,"Store registration mismatch");
                        let updated=factory.reembed(service,registration).await?;
                        services.lock().await.insert(request.store_path,updated);
                        return Ok(serde_json::json!({"status":"reembedded","embedding_ready":true,"memory_schema_version":2}));
                    }
                    match request.action {
                        Action::Write(value)=>Ok(serde_json::to_value(service.write(&request.context,value).await?)?),
                        Action::Read(value)=>Ok(serde_json::to_value(service.read(&request.context,value).await?)?),
                        Action::Update(value)=>service.update(&request.context,value).await,
                        Action::Delete(value)=>service.delete(&request.context,value).await,
                        Action::Status(id)=>Ok(serde_json::to_value(service.status(&request.context,&id).await?)?),
                        Action::Retry(id)=>Ok(serde_json::to_value(service.retry(&request.context,&id).await?)?),
                        Action::EmbedQuery(text)=>Ok(serde_json::to_value(service.embedder.query(&text).await?)?),
                        Action::GroundingChecks(checks)=>{service.grounding_checks(&request.context,checks).await?;Ok(serde_json::json!({"status":"checked"}))},
                        Action::ValidateReferences(references)=>{
                            let store = service.store.lock().await;
                            let valid: Vec<_> = references.into_iter().filter(|reference| store.state.claims.values().any(|claim|
                                request.context.authorizes(claim) && claim.lifecycle != Lifecycle::Retracted
                                && !store.state.tombstones.contains_key(&claim.id)
                                && *reference == format!("memory:{}@{}",claim.id,claim.revision))).collect();
                            Ok(serde_json::to_value(valid)?)
                        },
                        Action::ProjectionChanges{graph_project_id,cursor}=>{
                            let store=service.store.lock().await;
                            Ok(serde_json::to_value(store.projection_changes(&request.context,&graph_project_id,cursor).await?)?)
                        },
                        _=>bail!("Unsupported memory action"),
                    }
                }.await;
                let response=match result {Ok(value)=>Response{value:Some(value),error:None},Err(error)=>Response{value:None,error:Some(format!("{error:#}"))}};
                if let Ok(mut bytes)=serde_json::to_vec(&response){bytes.push(b'\n');let _=write.write_all(&bytes).await;}
            });
        }
        Ok(())
    }.await;
    worker.abort();
    let _ = worker.await;
    let _ = std::fs::remove_file(path);
    drop(lock);
    result
}
fn private_dir(path: &Path) -> Result<()> {
    if path.exists() {
        ensure!(
            !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
            "Memory runtime directory cannot be a symlink"
        );
    }
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
fn private_file(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "Invalid memory owner discovery file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "Memory owner discovery must be private"
        );
    }
    Ok(())
}
