//! Imports clients and suppliers from a JSON file through the normal write
//! path, so each one gets its shared code (`CL-…`, `SUP-…`) and syncs like
//! one typed into the app.
//!
//! ```text
//! cargo run -p sanco-core --example import_master -- --data-dir .dev/test \
//!     --file import.json [--workspace NAME]
//! ```
//!
//! The file is `{ "clients": [ClientInput…], "suppliers": [SupplierInput…] }`.
//! Names that already exist (ignoring case) are skipped, so a re-run adds
//! only what is missing. `--workspace` creates the workspace in a new
//! database; the first admin is then created in the app. Close any app
//! instance on that data dir first.

use std::path::PathBuf;

use sanco_core::domain::clients::{self, ClientInput};
use sanco_core::domain::suppliers::{self, SupplierInput};
use sanco_core::sync::numbering::{self, series};
use sanco_core::{Actor, Db, OpenOptions};
use serde::Deserialize;

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    clients: Vec<ClientInput>,
    #[serde(default)]
    suppliers: Vec<SupplierInput>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (mut dir, mut file, mut workspace) = (None, None, None);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--data-dir" => dir = Some(PathBuf::from(value()?)),
            "--file" => file = Some(PathBuf::from(value()?)),
            "--workspace" => workspace = Some(value()?),
            other => return Err(format!("unknown flag {other}").into()),
        }
    }
    let dir = dir.ok_or("--data-dir is required")?;
    let file = file.ok_or("--file is required")?;
    let data: File = serde_json::from_str(&std::fs::read_to_string(&file)?)?;

    std::fs::create_dir_all(&dir)?;
    let db = Db::open(OpenOptions::new(dir.join("sanco.db"), machine_id()))?;
    if let Some(name) = workspace
        && db.workspace()?.is_none()
    {
        db.create_workspace(&name)?;
        println!("created workspace {name}");
    }

    let existing = |table: &str| -> Result<Vec<String>, sanco_core::Error> {
        db.read(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT upper(trim(name)) FROM {table} WHERE _deleted = 0"
            ))?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    };

    let mut have = existing(clients::CLIENTS)?;
    let (mut added, mut skipped) = (0, 0);
    for input in data.clients {
        let name = input.name.trim().to_uppercase();
        if have.contains(&name) {
            skipped += 1;
            continue;
        }
        let id = db.new_id()?;
        let res = numbering::reserve(&db, &[], series::CLIENT, &id)?;
        db.write(Actor::system(), move |tx| {
            clients::create(tx, &input, &res, true)
        })?;
        have.push(name);
        added += 1;
    }
    println!("clients: {added} added, {skipped} already there");

    let mut have = existing(suppliers::SUPPLIERS)?;
    let (mut added, mut skipped) = (0, 0);
    for input in data.suppliers {
        let name = input.name.trim().to_uppercase();
        if have.contains(&name) {
            skipped += 1;
            continue;
        }
        let id = db.new_id()?;
        let res = numbering::reserve(&db, &[], series::SUPPLIER, &id)?;
        db.write(Actor::system(), move |tx| {
            suppliers::create(tx, &input, &res)
        })?;
        have.push(name);
        added += 1;
    }
    println!("suppliers: {added} added, {skipped} already there");
    Ok(())
}

/// Same id the app opens the database with (`MachineGuid`), so the app keeps
/// this database's site id instead of taking a new one.
fn machine_id() -> String {
    std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .last()
                .map(String::from)
        })
        .unwrap_or_else(|| "unknown".into())
}
