//! HTTP client side of the sync protocol. One call per endpoint in
//! [`crate::server`].

use std::path::Path;
use std::time::Duration;

use reqwest::{RequestBuilder, Response, StatusCode};
use sanco_core::sync::numbering::{ClaimReply, NumberClaim};
use sanco_core::sync::peer::{Hello, SnapshotInfo};
use sanco_core::sync::protocol::ChangeBatch;
use serde::de::DeserializeOwned;
use tokio::io::AsyncWriteExt;

use crate::error::{Error, Result};
use crate::server::{PokeBody, SNAPSHOT_HEADER};
use crate::token::Token;

#[derive(Clone)]
pub struct PeerClient {
    http: reqwest::Client,
    token: Token,
}

impl PeerClient {
    /// `timeout` bounds connecting and each whole request, except snapshot
    /// downloads, which only bound connecting.
    pub fn new(token: Token, timeout: Duration) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(timeout)
            .read_timeout(timeout)
            .no_proxy()
            .build()
            .expect("HTTP client config is valid");
        Self { http, token }
    }

    pub async fn hello(&self, addr: &str) -> Result<Hello> {
        self.json(addr, self.get(addr, "/hello")).await
    }

    pub async fn changes(&self, addr: &str, since: i64, limit: usize) -> Result<ChangeBatch> {
        let path = format!("/changes?since={since}&limit={limit}");
        self.json(addr, self.get(addr, &path)).await
    }

    /// Asks `addr` to pull from us soon. `port` is where we listen; the peer
    /// takes our IP from the connection.
    pub async fn poke(&self, addr: &str, port: u16, timeout: Duration) -> Result<()> {
        let req = self
            .post(addr, "/poke")
            .json(&PokeBody { port })
            .timeout(timeout);
        self.send(addr, req).await.map(drop)
    }

    pub async fn claim(
        &self,
        addr: &str,
        claim: &NumberClaim,
        timeout: Duration,
    ) -> Result<ClaimReply> {
        let req = self
            .post(addr, "/numbers/claim")
            .json(claim)
            .timeout(timeout);
        self.json(addr, req).await
    }

    /// Downloads a full copy of the peer's database to `dest`.
    pub async fn snapshot(&self, addr: &str, dest: &Path) -> Result<SnapshotInfo> {
        let mut res = self.send(addr, self.get(addr, "/snapshot")).await?;
        let info = res
            .headers()
            .get(SNAPSHOT_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| serde_json::from_str::<SnapshotInfo>(v).ok())
            .ok_or_else(|| Error::Protocol {
                addr: addr.into(),
                reason: "snapshot without info header".into(),
            })?;
        let mut file = tokio::fs::File::create(dest).await?;
        while let Some(chunk) = res.chunk().await.map_err(|e| unreachable(addr, e))? {
            file.write_all(&chunk).await?;
        }
        file.sync_all().await?;
        Ok(info)
    }

    fn get(&self, addr: &str, path: &str) -> RequestBuilder {
        self.http
            .get(format!("http://{addr}{path}"))
            .bearer_auth(self.token.as_str())
    }

    fn post(&self, addr: &str, path: &str) -> RequestBuilder {
        self.http
            .post(format!("http://{addr}{path}"))
            .bearer_auth(self.token.as_str())
    }

    async fn send(&self, addr: &str, req: RequestBuilder) -> Result<Response> {
        let res = req.send().await.map_err(|e| unreachable(addr, e))?;
        match res.status() {
            s if s.is_success() => Ok(res),
            StatusCode::UNAUTHORIZED => Err(Error::Unauthorized(addr.into())),
            s => {
                let body = res.text().await.unwrap_or_default();
                let message = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v["error"].as_str().map(String::from))
                    .unwrap_or(body);
                Err(Error::Status {
                    addr: addr.into(),
                    status: s.as_u16(),
                    message,
                })
            }
        }
    }

    async fn json<T: DeserializeOwned>(&self, addr: &str, req: RequestBuilder) -> Result<T> {
        let res = self.send(addr, req).await?;
        let bytes = res.bytes().await.map_err(|e| unreachable(addr, e))?;
        serde_json::from_slice(&bytes).map_err(|e| Error::Protocol {
            addr: addr.into(),
            reason: e.to_string(),
        })
    }
}

fn unreachable(addr: &str, e: reqwest::Error) -> Error {
    let reason = if e.is_timeout() {
        "timed out".to_string()
    } else if e.is_connect() {
        "connection failed".to_string()
    } else {
        e.to_string()
    };
    Error::Unreachable {
        addr: addr.into(),
        reason,
    }
}
