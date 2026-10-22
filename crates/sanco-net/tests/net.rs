//! Three PCs on loopback: join, sync, conflicts, handshake checks and
//! number claims over real HTTP.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

use rusqlite::OptionalExtension;
use sanco_core::sync::hlc::{Clock, SystemClock};
use sanco_core::sync::numbering::series;
use sanco_core::{Actor, Db, OpenOptions};
use sanco_net::client::PeerClient;
use sanco_net::token::SyncKey;
use sanco_net::{Error, JoinCode, LocalChanges, Node, NodeConfig, PeerState};
use serde_json::json;

/// System time plus an adjustable offset.
#[derive(Default)]
struct SkewClock(AtomicI64);

impl Clock for SkewClock {
    fn now_ms(&self) -> i64 {
        SystemClock.now_ms() + self.0.load(Ordering::SeqCst)
    }
}

struct Pc {
    name: &'static str,
    changes: LocalChanges,
    clock: Arc<SkewClock>,
    db: Arc<Db>,
    node: Option<Node>,
}

fn options(dir: &Path, name: &str, changes: &LocalChanges, clock: &Arc<SkewClock>) -> OpenOptions {
    let mut o = OpenOptions::new(dir.join(name).join("sanco.db"), name);
    o.durable = false;
    o.clock = clock.clone();
    o.on_change = Some(changes.listener(None));
    o
}

fn config(name: &str) -> NodeConfig {
    let mut cfg = NodeConfig::new(name, "0.1.0");
    cfg.bind_ip = [127, 0, 0, 1].into();
    cfg.port = 0;
    cfg.host = Some("127.0.0.1".into());
    // Long enough that tests see pokes and explicit syncs, not the timer.
    cfg.pull_interval = Duration::from_secs(600);
    cfg.poke_delay = Duration::from_millis(20);
    cfg.timeout = Duration::from_secs(2);
    cfg.claim_timeout = Duration::from_secs(1);
    cfg
}

impl Pc {
    async fn first(dir: &Path, name: &'static str) -> Pc {
        let changes = LocalChanges::new();
        let clock = Arc::new(SkewClock::default());
        let db = Db::open(options(dir, name, &changes, &clock)).unwrap();
        db.create_workspace("Sanco").unwrap();
        Pc::start(name, changes, clock, db).await
    }

    async fn join(dir: &Path, name: &'static str, code: &str) -> Pc {
        let changes = LocalChanges::new();
        let clock = Arc::new(SkewClock::default());
        let opts = options(dir, name, &changes, &clock);
        let db = sanco_net::join(code, opts, Duration::from_secs(5))
            .await
            .unwrap();
        Pc::start(name, changes, clock, db).await
    }

    async fn start(name: &'static str, changes: LocalChanges, clock: Arc<SkewClock>, db: Db) -> Pc {
        let mut pc = Pc {
            name,
            changes,
            clock,
            db: Arc::new(db),
            node: None,
        };
        pc.restart().await;
        pc
    }

    async fn restart(&mut self) {
        self.restart_with(config(self.name)).await;
    }

    async fn restart_with(&mut self, cfg: NodeConfig) {
        let node = Node::start(
            self.db.clone(),
            self.changes.clone(),
            cfg,
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
        self.node = Some(node);
    }

    async fn stop(&mut self) {
        self.node.take().unwrap().shutdown().await;
    }

    fn node(&self) -> &Node {
        self.node.as_ref().unwrap()
    }

    fn add_todo(&self, title: &str) -> String {
        let title = title.to_string();
        self.db
            .write(Actor::user("u1"), move |tx| {
                tx.insert("todos", json!({ "title": title }))
            })
            .unwrap()
    }

    fn rename(&self, id: &str, title: &str) {
        let (id, title) = (id.to_string(), title.to_string());
        self.db
            .write(Actor::user("u1"), move |tx| {
                tx.update("todos", &id, json!({ "title": title }))
            })
            .unwrap();
    }

    fn title(&self, id: &str) -> Option<String> {
        self.db
            .read(|c| {
                Ok(c.query_row(
                    "SELECT title FROM todos WHERE id = ?1 AND _deleted = 0",
                    [id],
                    |r| r.get(0),
                )
                .optional()?)
            })
            .unwrap()
    }

    fn count(&self, sql: &str) -> i64 {
        self.db
            .read(|c| Ok(c.query_row(sql, [], |r| r.get(0))?))
            .unwrap()
    }

    fn peer_state(&self, other: &Pc) -> Option<PeerState> {
        let site = other.db.site_id();
        self.node()
            .peers()
            .into_iter()
            .find(|p| p.site.as_deref() == Some(site))
            .map(|p| p.state)
    }
}

async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn trio(dir: &Path) -> (Pc, Pc, Pc) {
    let a = Pc::first(dir, "a").await;
    let code = a.node().invite_code().unwrap();
    let b = Pc::join(dir, "b", &code).await;
    let c = Pc::join(dir, "c", &code).await;
    // Pokes and device rows spread in the background; wait until every PC
    // has pulled from both others.
    wait_until("all PCs connected", || {
        [(&a, &b), (&a, &c), (&b, &a), (&b, &c), (&c, &a), (&c, &b)]
            .iter()
            .all(|(x, y)| x.peer_state(y) == Some(PeerState::Online))
    })
    .await;
    (a, b, c)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_pcs_join_and_sync_by_poke() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = trio(dir.path()).await;
    for pc in [&b, &c] {
        assert_eq!(pc.db.workspace().unwrap(), a.db.workspace().unwrap());
        assert_ne!(pc.db.site_id(), a.db.site_id());
    }

    // Every PC learns every device row, so they all find each other.
    wait_until("device rows", || {
        [&a, &b, &c]
            .iter()
            .all(|pc| pc.count("SELECT count(*) FROM devices WHERE _deleted = 0") == 3)
    })
    .await;

    let t = a.add_todo("from A");
    wait_until("A's todo on B and C", || {
        b.title(&t).is_some() && c.title(&t).is_some()
    })
    .await;
    let u = c.add_todo("from C");
    wait_until("C's todo on A and B", || {
        a.title(&u).is_some() && b.title(&u).is_some()
    })
    .await;

    for pc in [&a, &b, &c] {
        pc.node().sync_now().await;
        let peers = pc.node().peers();
        assert_eq!(peers.len(), 2, "{}: {peers:?}", pc.name);
        assert!(
            peers.iter().all(|p| p.state == PeerState::Online),
            "{peers:?}"
        );
    }
    assert_eq!(a.peer_state(&b), Some(PeerState::Online));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_key_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let a = Pc::first(dir.path(), "a").await;
    let addr = a.node().addr().unwrap();

    let other = SyncKey::from_bytes([9; 32]);
    let client = PeerClient::new(other.token(), Duration::from_secs(2));
    let err = client.hello(&addr).await.unwrap_err();
    assert!(matches!(err, Error::Unauthorized(_)), "{err}");

    let forged = JoinCode { key: other, addr }.encode();
    let opts = options(dir.path(), "x", &LocalChanges::new(), &Arc::default());
    let err = sanco_net::join(&forged, opts, Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Unauthorized(_)), "{err}");
    assert!(!dir.path().join("x").join("sanco.db").exists());

    let typo = a.node().invite_code().unwrap().replacen('A', "B", 1);
    let opts = options(dir.path(), "y", &LocalChanges::new(), &Arc::default());
    if typo != a.node().invite_code().unwrap() {
        let err = sanco_net::join(&typo, opts, Duration::from_secs(2))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::BadJoinCode(_)), "{err}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_edits_become_a_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let a = Pc::first(dir.path(), "a").await;
    let mut b = Pc::join(dir.path(), "b", &a.node().invite_code().unwrap()).await;
    let t = a.add_todo("draft");
    wait_until("todo on B", || b.title(&t).is_some()).await;
    wait_until("B known to A", || {
        a.peer_state(&b) == Some(PeerState::Online)
    })
    .await;

    b.stop().await;
    a.rename(&t, "A's edit");
    b.rename(&t, "B's edit");
    a.node().sync_now().await;
    assert_eq!(a.peer_state(&b), Some(PeerState::Offline));

    // B comes back (on a new port) and pokes A; both pull.
    b.restart().await;
    wait_until("both edits merged", || {
        a.title(&t) == b.title(&t)
            && a.count("SELECT count(*) FROM conflicts") == 1
            && b.count("SELECT count(*) FROM conflicts") == 1
    })
    .await;
    a.node().sync_now().await;
    assert_eq!(a.peer_state(&b), Some(PeerState::Online));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn skewed_clock_is_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let a = Pc::first(dir.path(), "a").await;
    let code = a.node().invite_code().unwrap();

    // Joining with a bad clock fails before anything is downloaded.
    let clock = Arc::new(SkewClock(AtomicI64::new(5 * 60 * 1000)));
    let opts = options(dir.path(), "late", &LocalChanges::new(), &clock);
    let err = sanco_net::join(&code, opts, Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Core(sanco_core::Error::ClockSkew { .. })),
        "{err}"
    );

    // A PC whose clock drifts after joining is blocked, not merged.
    let b = Pc::join(dir.path(), "b", &code).await;
    wait_until("B known to A", || {
        a.peer_state(&b) == Some(PeerState::Online)
    })
    .await;
    b.clock.0.store(-5 * 60 * 1000, Ordering::SeqCst);
    let t = b.add_todo("from the past");
    a.node().sync_now().await;
    assert_eq!(a.peer_state(&b), Some(PeerState::Blocked));
    let err = a.node().peers().into_iter().find_map(|p| p.error).unwrap();
    assert!(err.contains("behind"), "{err}");
    assert_eq!(a.title(&t), None);

    b.clock.0.store(0, Ordering::SeqCst);
    a.node().sync_now().await;
    assert_eq!(a.peer_state(&b), Some(PeerState::Online));
    assert!(a.title(&t).is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn numbers_are_claimed_over_http() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, mut c) = trio(dir.path()).await;
    for pc in [&a, &b, &c] {
        pc.node().sync_now().await;
    }

    let first = a.node().reserve_number(series::DR, "doc-1").await.unwrap();
    assert!(first.confirmed());
    assert_eq!((first.reached, first.seq), (2, 1));
    // B hasn't seen the document yet, but A's claim holds the number.
    let second = b.node().reserve_number(series::DR, "doc-2").await.unwrap();
    assert!(second.confirmed());
    assert_eq!(second.seq, 2);

    c.stop().await;
    a.node().sync_now().await;
    let started = Instant::now();
    let third = a.node().reserve_number(series::DR, "doc-3").await.unwrap();
    assert!(!third.confirmed());
    assert_eq!((third.reached, third.unreachable, third.seq), (1, 1, 3));
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "offline peer was waited for"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn newer_peer_shares_its_installer() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = Pc::first(dir.path(), "a").await;
    let code = a.node().invite_code().unwrap();
    let mut b = Pc::join(dir.path(), "b", &code).await;
    wait_until("connected", || b.peer_state(&a) == Some(PeerState::Online)).await;
    assert_eq!(b.node().update_offer(), None);

    // A was updated to 0.2.0 and has its installer; B still runs 0.1.0.
    let installer = dir.path().join("sanco-setup.exe");
    let exe = b"MZ pretend this is Sanco 0.2.0".to_vec();
    std::fs::write(&installer, &exe).unwrap();
    let port = a.node().port();
    a.stop().await;
    let mut cfg = config("a");
    cfg.port = port;
    cfg.app_version = "0.2.0".into();
    cfg.installer = Some(installer.clone());
    a.restart_with(cfg).await;
    b.node().sync_now().await;
    let offer = b.node().update_offer().expect("an update offer");
    assert_eq!(offer.version, "0.2.0");
    assert_eq!(offer.name.as_deref(), Some("a"));
    // A doesn't offer B's older version.
    a.node().sync_now().await;
    assert_eq!(a.node().update_offer(), None);

    let path = b
        .node()
        .download_update(&offer.addr, &dir.path().join("updates"))
        .await
        .unwrap();
    assert!(path.ends_with("Sanco_0.2.0_x64-setup.exe"));
    assert_eq!(std::fs::read(&path).unwrap(), exe);
    // B can't take an "update" from a PC with the same or an older version.
    let b_addr = b.node().addr().unwrap();
    assert!(a.node().download_update(&b_addr, dir.path()).await.is_err());

    // Without an installer file, the offer stays but the download explains.
    std::fs::remove_file(&installer).unwrap();
    let err = b
        .node()
        .download_update(&offer.addr, &dir.path().join("updates"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no installer"), "{err}");
    b.stop().await;
}
