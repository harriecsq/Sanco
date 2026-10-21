//! Headless sync node for testing next to app instances.
//!
//! ```text
//! cargo run -p sanco-net --example node -- --data-dir .dev/x --port 7480 --host 127.0.0.1 \
//!     [--create NAME | --join CODE] [--note TEXT]
//! ```
//!
//! Prints the join code, then every few seconds the peers and how many
//! to-dos it has. `--note` adds a to-do once it is running. Ctrl+C stops it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sanco_core::{Actor, Db, OpenOptions};
use sanco_net::{LocalChanges, Node, NodeConfig};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (mut dir, mut port, mut host) = (PathBuf::from(".dev/node"), 7480u16, None);
    let (mut create, mut join, mut note) = (None, None, None);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--data-dir" => dir = value()?.into(),
            "--port" => port = value()?.parse()?,
            "--host" => host = Some(value()?),
            "--create" => create = Some(value()?),
            "--join" => join = Some(value()?),
            "--note" => note = Some(value()?),
            other => return Err(format!("unknown flag {other}").into()),
        }
    }

    let changes = LocalChanges::new();
    let mut opts = OpenOptions::new(dir.join("sanco.db"), "headless-node");
    opts.on_change = Some(changes.listener(None));
    let db = match join {
        Some(code) => sanco_net::join(&code, opts, Duration::from_secs(15)).await?,
        None => Db::open(opts)?,
    };
    if let Some(name) = create {
        db.create_workspace(&name)?;
    }

    let mut cfg = NodeConfig::new(format!("node-{port}"), "dev");
    cfg.port = port;
    cfg.host = host;
    let node = Node::start(Arc::new(db), changes, cfg, Arc::new(|_| {})).await?;
    println!("site {} listening on {}", node.db().site_id(), node.port());
    match node.invite_code() {
        Ok(code) => println!("join code: {code}"),
        Err(e) => println!("no join code: {e}"),
    }
    if let Some(title) = note {
        node.db().write(Actor::system(), move |tx| {
            tx.insert("todos", json!({ "title": title }))
        })?;
    }

    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
            _ = tokio::signal::ctrl_c() => break,
        }
        let todos: i64 = node.db().read(|c| {
            Ok(
                c.query_row("SELECT count(*) FROM todos WHERE _deleted = 0", [], |r| {
                    r.get(0)
                })?,
            )
        })?;
        let peers: Vec<String> = node
            .peers()
            .iter()
            .map(|p| format!("{} {:?} {:?}", p.addr, p.state, p.name))
            .collect();
        println!("todos {todos} | peers {peers:?}");
    }
    node.shutdown().await;
    Ok(())
}
