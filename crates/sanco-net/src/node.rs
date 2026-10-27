//! A running sync node: the server plus the loops that pull from and poke
//! peers.
//!
//! - Every `pull_interval` (30 s) it pulls from every known peer.
//! - After each local commit (debounced) it pokes peers, which pull from it.
//! - When poked, it pulls from the caller right away.
//!
//! Peers come from the replicated `devices` table plus any address that
//! poked us. Each pull starts with a handshake ([`Db::check_peer`]); a peer
//! that fails it is shown as blocked and nothing is pulled from it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use sanco_core::db::migrations::SCHEMA_VERSION;
use sanco_core::domain::boxes::{SerialNeed, Serials};
use sanco_core::db::registry::DOC_NUMBERS;
use sanco_core::repo::Listener;
use sanco_core::sync::hlc::MAX_SKEW_MS;
use sanco_core::sync::numbering::{
    self, ClaimReply, NumberClaim, NumberPeer, Renumbered, Reservation,
};
use sanco_core::sync::peer::{DeviceInfo, Hello, join_host_port};
use sanco_core::sync::protocol::ApplyReport;
use sanco_core::{ChangeEvent, Db, OpenOptions, Origin};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::runtime::Handle;
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;

use crate::client::PeerClient;
use crate::error::{Error, Result};
use crate::joincode::JoinCode;
use crate::server;
use crate::tailscale;
use crate::token::{SyncKey, Token};
use crate::update::{self, UpdateOffer};

/// How often the device row and Tailscale address are re-checked.
const DEVICE_CHECK: Duration = Duration::from_secs(10 * 60);

pub const DEFAULT_PORT: u16 = 7470;

#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub bind_ip: IpAddr,
    /// 0 picks a free port (tests).
    pub port: u16,
    /// Address peers should use for this PC. `None` asks Tailscale.
    pub host: Option<String>,
    /// This PC's display name.
    pub name: String,
    pub app_version: String,
    pub pull_interval: Duration,
    /// Wait after a local commit before poking, so bursts become one poke.
    pub poke_delay: Duration,
    /// Connect/read timeout for sync requests.
    pub timeout: Duration,
    /// Timeout for number claims and pokes.
    pub claim_timeout: Duration,
    /// Rows per pulled batch.
    pub batch_limit: usize,
    /// This version's installer, shared with peers that run an older
    /// version (`GET /update`). `None` when there is none (dev builds).
    pub installer: Option<std::path::PathBuf>,
}

impl NodeConfig {
    pub fn new(name: impl Into<String>, app_version: impl Into<String>) -> Self {
        Self {
            bind_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: DEFAULT_PORT,
            host: None,
            name: name.into(),
            app_version: app_version.into(),
            pull_interval: Duration::from_secs(30),
            poke_delay: Duration::from_millis(300),
            timeout: Duration::from_secs(5),
            claim_timeout: Duration::from_secs(2),
            batch_limit: 500,
            installer: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PeerState {
    /// Not contacted yet.
    Unknown,
    Online,
    Offline,
    /// Reachable but refused: wrong key, other schema, bad clock, restored
    /// database. Needs someone to act.
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PeerStatus {
    pub addr: String,
    pub site: Option<String>,
    pub name: Option<String>,
    pub app_version: Option<String>,
    pub state: PeerState,
    pub error: Option<String>,
    /// Last successful pull (UTC ms).
    pub last_pull_ms: Option<i64>,
    /// Rows accepted and conflicts found by the last pull.
    pub last_accepted: usize,
    pub last_conflicts: usize,
}

impl PeerStatus {
    fn new(addr: &str) -> Self {
        Self {
            addr: addr.into(),
            site: None,
            name: None,
            app_version: None,
            state: PeerState::Unknown,
            error: None,
            last_pull_ms: None,
            last_accepted: 0,
            last_conflicts: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum NodeEvent {
    /// Peer statuses changed; see [`Node::peers`].
    PeersChanged,
    /// Documents this PC numbered lost a clash and got new numbers.
    Renumbered(Vec<Renumbered>),
    /// An admin removed this PC from the workspace. The shell should stop
    /// the node; this PC can only come back by joining again.
    Removed,
}

pub type EventFn = Arc<dyn Fn(NodeEvent) + Send + Sync>;

/// Wakes the poke loop when this PC commits. Create it before opening the
/// database and pass [`LocalChanges::listener`] as `OpenOptions::on_change`.
#[derive(Clone, Default)]
pub struct LocalChanges(Arc<Notify>);

impl LocalChanges {
    pub fn new() -> Self {
        Self::default()
    }

    /// A change listener that wakes the poke loop on local commits, then
    /// calls `then` (e.g. to tell the UI to refetch).
    pub fn listener(&self, then: Option<Listener>) -> Listener {
        let notify = self.0.clone();
        Arc::new(move |ev: &ChangeEvent| {
            if ev.origin == Origin::Local {
                notify.notify_one();
            }
            if let Some(f) = &then {
                f(ev);
            }
        })
    }
}

/// State shared by the server, the loops and [`Node`].
pub(crate) struct Inner {
    pub(crate) db: Arc<Db>,
    pub(crate) cfg: NodeConfig,
    pub(crate) token: Token,
    client: PeerClient,
    port: u16,
    rt: Handle,
    host: Mutex<Option<String>>,
    peers: Mutex<BTreeMap<String, PeerStatus>>,
    /// Addresses that poked us, in case their device row hasn't arrived.
    learned: Mutex<BTreeSet<String>>,
    pull_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Wakes the poke loop: local commits, and rows accepted from a peer
    /// (so changes relay on without waiting for the timer).
    poke: Arc<Notify>,
    on_event: EventFn,
}

pub struct Node {
    inner: Arc<Inner>,
    stop: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl Node {
    /// Starts the server and sync loops. The database must belong to a
    /// workspace. Must be called inside a Tokio runtime.
    pub async fn start(
        db: Arc<Db>,
        changes: LocalChanges,
        cfg: NodeConfig,
        on_event: EventFn,
    ) -> Result<Node> {
        let ws = db.workspace()?.ok_or(sanco_core::Error::NoWorkspace)?;
        let token = SyncKey::from_hex(&ws.sync_key)?.token();
        let listener = TcpListener::bind((cfg.bind_ip, cfg.port)).await?;
        let port = listener.local_addr()?.port();
        let inner = Arc::new(Inner {
            db,
            client: PeerClient::new(token.clone(), cfg.timeout),
            token,
            host: Mutex::new(cfg.host.clone()),
            cfg,
            port,
            rt: Handle::current(),
            peers: Mutex::default(),
            learned: Mutex::default(),
            pull_locks: Mutex::default(),
            poke: changes.0.clone(),
            on_event,
        });
        tracing::info!(port, site = inner.db.site_id(), "sync server listening");

        let (stop, stop_rx) = watch::channel(false);
        let app = server::router(inner.clone()).into_make_service_with_connect_info::<SocketAddr>();
        let mut rx = stop_rx.clone();
        let server = tokio::spawn(async move {
            let shutdown = async move {
                let _ = rx.wait_for(|s| *s).await;
            };
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await
            {
                tracing::error!(error = %e, "sync server stopped");
            }
        });

        inner.refresh_device().await;
        // Tell peers we're here, so they pull our device row and changes.
        inner.poke.notify_one();
        let pull = tokio::spawn(inner.clone().pull_loop(stop_rx.clone()));
        let poke = tokio::spawn(inner.clone().poke_loop(stop_rx));
        Ok(Node {
            inner,
            stop,
            tasks: vec![server, pull, poke],
        })
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.inner.db
    }

    /// Port the server listens on.
    pub fn port(&self) -> u16 {
        self.inner.port
    }

    /// `host:port` peers use for this PC, if the host is known.
    pub fn addr(&self) -> Option<String> {
        self.inner.own_addr()
    }

    /// Code another PC enters to join this workspace. Contains the sync key:
    /// share it only with people who should get all the data.
    pub fn invite_code(&self) -> Result<String> {
        let addr = self.addr().ok_or(Error::NoAddress)?;
        let ws = self
            .inner
            .db
            .workspace()?
            .ok_or(sanco_core::Error::NoWorkspace)?;
        Ok(JoinCode {
            key: SyncKey::from_hex(&ws.sync_key)?,
            addr,
        }
        .encode())
    }

    pub fn peers(&self) -> Vec<PeerStatus> {
        self.inner.peers().values().cloned().collect()
    }

    /// Pulls from every peer now and waits until done.
    pub async fn sync_now(&self) {
        self.inner.pull_all().await;
    }

    /// Reserves the next number of `series` for document `ref_id` with every
    /// reachable peer. See [`numbering::reserve`].
    pub async fn reserve_number(&self, series: &str, ref_id: &str) -> Result<Reservation> {
        let peers = self.inner.number_peers().await?;
        let (series, ref_id) = (series.to_string(), ref_id.to_string());
        self.inner
            .blocking(move |db| {
                let refs: Vec<&dyn NumberPeer> =
                    peers.iter().map(|p| p as &dyn NumberPeer).collect();
                numbering::reserve(db, &refs, &series, &ref_id)
            })
            .await
    }

    /// Reserves the box serials a write needs (`ref_id` is the document
    /// that makes the boxes) with every reachable peer. See
    /// [`numbering::reserve_batch`].
    pub async fn reserve_serials(&self, needs: Vec<SerialNeed>, ref_id: &str) -> Result<Serials> {
        if needs.is_empty() {
            return Ok(Serials::none());
        }
        let peers = self.inner.number_peers().await?;
        let ref_id = ref_id.to_string();
        self.inner
            .blocking(move |db| {
                let refs: Vec<&dyn NumberPeer> =
                    peers.iter().map(|p| p as &dyn NumberPeer).collect();
                let batches = needs
                    .iter()
                    .map(|n| {
                        numbering::reserve_batch(db, &refs, &n.series, &n.period, &ref_id, n.count)
                    })
                    .collect::<sanco_core::Result<Vec<_>>>()?;
                Ok(Serials::new(batches))
            })
            .await
    }

    /// Claims a number typed by staff for document `ref_id` with every
    /// reachable peer. See [`numbering::reserve_exact`].
    pub async fn reserve_exact(&self, series: &str, no: &str, ref_id: &str) -> Result<Reservation> {
        let peers = self.inner.number_peers().await?;
        let (series, no, ref_id) = (series.to_string(), no.to_string(), ref_id.to_string());
        self.inner
            .blocking(move |db| {
                let refs: Vec<&dyn NumberPeer> =
                    peers.iter().map(|p| p as &dyn NumberPeer).collect();
                numbering::reserve_exact(db, &refs, &series, &no, &ref_id)
            })
            .await
    }

    /// The newest version a known peer runs, if newer than ours. Peers seen
    /// recently (online or blocked) come before offline ones.
    pub fn update_offer(&self) -> Option<UpdateOffer> {
        let ours = &self.inner.cfg.app_version;
        self.peers()
            .into_iter()
            .filter_map(|p| {
                let v = p.app_version.clone()?;
                update::is_newer(&v, ours).then_some((p, v))
            })
            .max_by(|(a, va), (b, vb)| {
                update::compare(va, vb).then_with(|| {
                    let reachable = |p: &PeerStatus| p.state != PeerState::Offline;
                    reachable(a).cmp(&reachable(b))
                })
            })
            .map(|(p, version)| UpdateOffer {
                version,
                addr: p.addr,
                name: p.name,
            })
    }

    /// Downloads the installer of a newer version from `addr` into `dir`
    /// and returns its path. Refuses anything not newer than this app.
    pub async fn download_update(&self, addr: &str, dir: &std::path::Path) -> Result<std::path::PathBuf> {
        let client = PeerClient::new(self.inner.token.clone(), Duration::from_secs(30));
        let info = client.update_info(addr).await?.ok_or_else(|| Error::Status {
            addr: addr.into(),
            status: 404,
            message: "that PC has no installer to share (it was not installed with the setup program)".into(),
        })?;
        if !update::is_newer(&info.version, &self.inner.cfg.app_version) {
            return Err(Error::Protocol {
                addr: addr.into(),
                reason: format!("its installer is version {}, not newer", info.version),
            });
        }
        let bytes = client.installer(addr, &info).await?;
        tokio::fs::create_dir_all(dir).await?;
        let name = format!("Sanco_{}_x64-setup.exe", info.version.replace(['/', '\\', ':'], "_"));
        let path = dir.join(name);
        let tmp = path.with_extension("partial");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        tracing::info!(version = %info.version, %addr, "downloaded an update");
        Ok(path)
    }

    /// Stops the server and loops, and waits for them.
    pub async fn shutdown(mut self) {
        let _ = self.stop.send(true);
        for t in self.tasks.drain(..) {
            let _ = t.await;
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

impl Inner {
    /// Runs blocking database work off the async threads.
    pub(crate) async fn blocking<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Db) -> sanco_core::Result<T> + Send + 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || f(&db))
            .await
            .map_err(|e| Error::Task(e.to_string()))?
            .map_err(Error::from)
    }

    fn own_addr(&self) -> Option<String> {
        let host = lock(&self.host).clone()?;
        Some(join_host_port(&host, self.port.into()))
    }

    fn peers(&self) -> MutexGuard<'_, BTreeMap<String, PeerStatus>> {
        lock(&self.peers)
    }

    fn emit(&self, ev: NodeEvent) {
        (self.on_event)(ev);
    }

    pub(crate) fn learn(&self, addr: &str) {
        if Some(addr) != self.own_addr().as_deref() {
            lock(&self.learned).insert(addr.to_string());
        }
    }

    /// Re-detects our address and writes our `devices` row if it changed.
    async fn refresh_device(&self) {
        let host = match &self.cfg.host {
            Some(h) => Some(h.clone()),
            None => tokio::task::spawn_blocking(tailscale::detect_ip)
                .await
                .ok()
                .flatten(),
        };
        if host.is_none() {
            tracing::warn!(
                "Tailscale address unknown; peers can only find this PC after it pokes them"
            );
        }
        *lock(&self.host) = host.clone();
        let info = DeviceInfo {
            name: self.cfg.name.clone(),
            host,
            port: self.port,
            app_version: self.cfg.app_version.clone(),
        };
        match self.blocking(move |db| db.register_device(&info)).await {
            Ok(_) => {}
            Err(Error::Core(sanco_core::Error::Deleted { .. })) => {
                tracing::warn!("this PC was removed from the workspace's devices");
                self.emit(NodeEvent::Removed);
            }
            Err(e) => tracing::error!(error = %e, "could not register this device"),
        }
    }

    /// Addresses of every other PC we know of: device rows, plus addresses
    /// that poked us whose device row hasn't arrived yet.
    async fn targets(&self) -> Vec<String> {
        let own_site = self.db.site_id().to_string();
        let devices = self.blocking(|db| db.devices()).await.unwrap_or_default();
        let others = devices.iter().filter(|d| d.site != own_site);
        let device_sites: BTreeSet<&str> = others.clone().map(|d| d.site.as_str()).collect();
        let mut set: BTreeSet<String> = others.filter_map(|d| d.addr()).collect();

        let mut peers = self.peers();
        {
            let mut learned = lock(&self.learned);
            learned.retain(|addr| {
                let site = peers.get(addr).and_then(|p| p.site.as_deref());
                !set.contains(addr) && site.is_none_or(|s| !device_sites.contains(s))
            });
            set.extend(learned.iter().cloned());
        }
        if let Some(own) = self.own_addr() {
            set.remove(&own);
        }
        peers.retain(|addr, _| set.contains(addr));
        for addr in &set {
            peers
                .entry(addr.clone())
                .or_insert_with(|| PeerStatus::new(addr));
        }
        set.into_iter().collect()
    }

    async fn pull_loop(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let mut tick = tokio::time::interval(self.cfg.pull_interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut device_checked = Instant::now();
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                _ = stop.wait_for(|s| *s) => break,
            }
            if device_checked.elapsed() >= DEVICE_CHECK {
                self.refresh_device().await;
                device_checked = Instant::now();
            }
            self.pull_all().await;
        }
    }

    async fn poke_loop(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        loop {
            tokio::select! {
                _ = self.poke.notified() => {}
                _ = stop.wait_for(|s| *s) => break,
            }
            tokio::time::sleep(self.cfg.poke_delay).await;
            self.poke_all().await;
        }
    }

    async fn poke_all(self: &Arc<Self>) {
        let mut set = tokio::task::JoinSet::new();
        for addr in self.targets().await {
            let me = self.clone();
            set.spawn(async move {
                let res = me.client.poke(&addr, me.port, me.cfg.claim_timeout).await;
                if let Err(e) = res {
                    tracing::debug!(%addr, error = %e, "poke failed");
                    me.record_error(&addr, &e);
                }
            });
        }
        set.join_all().await;
        self.emit(NodeEvent::PeersChanged);
    }

    pub(crate) async fn pull_all(self: &Arc<Self>) {
        let mut set = tokio::task::JoinSet::new();
        for addr in self.targets().await {
            let me = self.clone();
            set.spawn(async move { me.pull_peer(&addr).await });
        }
        set.join_all().await;
    }

    fn pull_lock(&self, addr: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.pull_locks)
            .entry(addr.to_string())
            .or_default()
            .clone()
    }

    /// Handshake, then pulls batches from `addr` until it has nothing new.
    pub(crate) async fn pull_peer(self: &Arc<Self>, addr: &str) {
        let numbers_changed = {
            let lock = self.pull_lock(addr);
            let _one_pull_per_peer = lock.lock().await;
            match self.pull_from(addr).await {
                Ok((hello, report, numbers)) => {
                    if report.accepted > 0 {
                        self.poke.notify_one();
                    }
                    if report.accepted > 0 && self.db.is_removed().unwrap_or(false) {
                        self.emit(NodeEvent::Removed);
                    }
                    self.record_ok(addr, &hello, report);
                    numbers
                }
                Err(e) => {
                    tracing::debug!(%addr, error = %e, "pull failed");
                    self.record_error(addr, &e);
                    false
                }
            }
        };
        self.emit(NodeEvent::PeersChanged);
        if numbers_changed {
            self.resolve_clashes().await;
        }
    }

    async fn pull_from(&self, addr: &str) -> Result<(Hello, ApplyReport, bool)> {
        let hello = self.client.hello(addr).await?;
        self.note_hello(addr, &hello);
        let h = hello.clone();
        self.blocking(move |db| db.check_peer(&h)).await?;

        let mut total = ApplyReport::default();
        let mut numbers = false;
        loop {
            let site = hello.site.clone();
            let since = self.blocking(move |db| db.watermark(&site)).await?;
            let batch = self
                .client
                .changes(addr, since, self.cfg.batch_limit)
                .await?;
            if batch.site != hello.site {
                return Err(Error::Protocol {
                    addr: addr.into(),
                    reason: "site changed during the pull".into(),
                });
            }
            let more = batch.more;
            let has_numbers = batch.rows.iter().any(|r| r.t == DOC_NUMBERS);
            let report = self.blocking(move |db| db.apply_batch(batch)).await?;
            numbers |= has_numbers && report.accepted > 0;
            total.accepted += report.accepted;
            total.ignored += report.ignored;
            total.conflicts += report.conflicts;
            if !more {
                break;
            }
        }
        if total.accepted > 0 {
            tracing::info!(%addr, accepted = total.accepted, conflicts = total.conflicts, "pulled");
        }
        Ok((hello, total, numbers))
    }

    async fn resolve_clashes(self: &Arc<Self>) {
        let peers = match self.number_peers().await {
            Ok(p) => p,
            Err(e) => return tracing::error!(error = %e, "number peers"),
        };
        let res = self
            .blocking(move |db| {
                let refs: Vec<&dyn NumberPeer> =
                    peers.iter().map(|p| p as &dyn NumberPeer).collect();
                numbering::resolve_clashes(db, &refs)
            })
            .await;
        match res {
            Ok(done) if !done.is_empty() => self.emit(NodeEvent::Renumbered(done)),
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "resolving number clashes failed"),
        }
    }

    /// Every known peer for number claims. Peers known to be offline or
    /// blocked fail at once instead of waiting for a timeout.
    async fn number_peers(self: &Arc<Self>) -> Result<Vec<HttpPeer>> {
        let targets = self.targets().await;
        let peers = self.peers();
        Ok(targets
            .into_iter()
            .map(|addr| {
                let usable = peers
                    .get(&addr)
                    .is_none_or(|p| matches!(p.state, PeerState::Online | PeerState::Unknown));
                HttpPeer {
                    inner: self.clone(),
                    addr,
                    usable,
                }
            })
            .collect())
    }

    /// Remembers who answers at `addr` and its version, even if the pull is
    /// then refused (another schema needs exactly this to offer an update).
    fn note_hello(&self, addr: &str, hello: &Hello) {
        let mut peers = self.peers();
        let p = peers
            .entry(addr.to_string())
            .or_insert_with(|| PeerStatus::new(addr));
        p.site = Some(hello.site.clone());
        p.name = Some(hello.name.clone());
        p.app_version = Some(hello.app_version.clone());
    }

    fn record_ok(&self, addr: &str, hello: &Hello, report: ApplyReport) {
        let now = self.db.now_ms();
        let mut peers = self.peers();
        let p = peers
            .entry(addr.to_string())
            .or_insert_with(|| PeerStatus::new(addr));
        p.site = Some(hello.site.clone());
        p.name = Some(hello.name.clone());
        p.app_version = Some(hello.app_version.clone());
        p.state = PeerState::Online;
        p.error = None;
        p.last_pull_ms = Some(now);
        p.last_accepted = report.accepted;
        p.last_conflicts = report.conflicts;
    }

    fn record_error(&self, addr: &str, e: &Error) {
        let mut peers = self.peers();
        let p = peers
            .entry(addr.to_string())
            .or_insert_with(|| PeerStatus::new(addr));
        p.state = if e.is_blocking() {
            PeerState::Blocked
        } else {
            PeerState::Offline
        };
        p.error = Some(e.to_string());
        drop(peers);
        // A poker we can't pull from will poke again when it's back.
        lock(&self.learned).remove(addr);
    }
}

/// A peer answering number claims over HTTP. Its `claim` blocks, so use it
/// only from blocking threads (as `Node::reserve_number` does).
struct HttpPeer {
    inner: Arc<Inner>,
    addr: String,
    usable: bool,
}

impl NumberPeer for HttpPeer {
    fn claim(&self, claim: &NumberClaim) -> sanco_core::Result<ClaimReply> {
        let unreachable = || sanco_core::Error::PeerUnreachable(self.addr.clone());
        if !self.usable {
            return Err(unreachable());
        }
        let inner = &self.inner;
        inner
            .rt
            .block_on(
                inner
                    .client
                    .claim(&self.addr, claim, inner.cfg.claim_timeout),
            )
            .map_err(|e| {
                tracing::debug!(addr = %self.addr, error = %e, "number claim failed");
                inner.record_error(&self.addr, &e);
                unreachable()
            })
    }
}

/// Sets up this PC from a join code: downloads a snapshot from the inviting
/// PC to `opts.path` and opens it. The caller then starts a [`Node`].
pub async fn join(code: &str, opts: OpenOptions, timeout: Duration) -> Result<Db> {
    let code = JoinCode::decode(code)?;
    let path = opts.path.clone();
    if path.exists() {
        return Err(Error::AlreadySetUp(path.display().to_string()));
    }
    let client = PeerClient::new(code.key.token(), timeout);
    let hello = client.hello(&code.addr).await?;
    if hello.schema != SCHEMA_VERSION {
        return Err(sanco_core::Error::SchemaMismatch {
            ours: SCHEMA_VERSION,
            theirs: hello.schema,
        }
        .into());
    }
    let ahead_ms = hello.now_ms - opts.clock.now_ms();
    if ahead_ms.abs() > MAX_SKEW_MS {
        return Err(sanco_core::Error::ClockSkew { ahead_ms }.into());
    }

    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let tmp = path.with_extension("joining");
    let _ = tokio::fs::remove_file(&tmp).await;
    let info = match client.snapshot(&code.addr, &tmp).await {
        Ok(info) => info,
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e);
        }
    };
    tokio::fs::rename(&tmp, &path).await?;
    tracing::info!(from = %code.addr, head = info.head, "joined workspace from snapshot");

    let db = tokio::task::spawn_blocking(move || Db::open(opts))
        .await
        .map_err(|e| Error::Task(e.to_string()))??;
    db.finish_join(&info)?;
    Ok(db)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
