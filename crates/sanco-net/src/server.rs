//! HTTP server every PC runs (axum on `0.0.0.0:PORT`).
//!
//! Every request must come from a Tailscale address (or loopback, for
//! several instances on one PC) and carry the workspace bearer token.
//!
//! | Route | Body / reply |
//! |---|---|
//! | `GET /hello` | → [`Hello`](sanco_core::sync::peer::Hello) |
//! | `GET /changes?since=N&limit=M` | → [`ChangeBatch`](sanco_core::sync::protocol::ChangeBatch) |
//! | `POST /poke` | [`PokeBody`] → 204; we pull from the caller soon |
//! | `POST /numbers/claim` | [`NumberClaim`] → [`ClaimReply`](sanco_core::sync::numbering::ClaimReply) |
//! | `GET /snapshot` | → database file, info in the [`SNAPSHOT_HEADER`] header |
//! | `GET /update` | → [`UpdateInfo`](crate::UpdateInfo) of this version's installer, or 404 |
//! | `GET /update/installer` | → the installer file |

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use sanco_core::sync::numbering::NumberClaim;
use sanco_core::sync::peer::join_host_port;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::Error;
use crate::node::Inner;
use crate::tailscale;
use crate::update;

/// JSON [`SnapshotInfo`](sanco_core::sync::peer::SnapshotInfo) of a snapshot download.
pub const SNAPSHOT_HEADER: &str = "x-sanco-snapshot";
const MAX_BATCH: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PokeBody {
    /// Port the caller listens on.
    pub port: u16,
}

#[derive(Deserialize)]
struct ChangesQuery {
    since: i64,
    limit: Option<usize>,
}

pub(crate) fn router(node: Arc<Inner>) -> Router {
    Router::new()
        .route("/hello", get(hello))
        .route("/changes", get(changes))
        .route("/poke", post(poke))
        .route("/numbers/claim", post(claim))
        .route("/snapshot", get(snapshot))
        .route("/update", get(update_info))
        .route("/update/installer", get(update_installer))
        .layer(middleware::from_fn_with_state(node.clone(), guard))
        .with_state(node)
}

/// Source address and token check for every route.
async fn guard(
    State(node): State<Arc<Inner>>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    if !tailscale::is_allowed_peer(remote.ip()) {
        tracing::warn!(%remote, "refused: not a Tailscale address");
        return reject(
            StatusCode::FORBIDDEN,
            "only Tailscale addresses may connect",
        );
    }
    let authorized = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|h| node.token.matches_header(h));
    if !authorized {
        tracing::warn!(%remote, "refused: wrong sync token");
        return reject(StatusCode::UNAUTHORIZED, "wrong sync key");
    }
    next.run(req).await
}

async fn hello(State(node): State<Arc<Inner>>) -> Result<Response, ApiError> {
    let (name, version) = (node.cfg.name.clone(), node.cfg.app_version.clone());
    let mut hello = node.blocking(move |db| db.hello(&name, &version)).await?;
    hello.os.clone_from(&node.cfg.os);
    Ok(Json(hello).into_response())
}

async fn changes(
    State(node): State<Arc<Inner>>,
    Query(q): Query<ChangesQuery>,
) -> Result<Response, ApiError> {
    let limit = q.limit.unwrap_or(node.cfg.batch_limit).clamp(1, MAX_BATCH);
    let batch = node
        .blocking(move |db| db.changes_since(q.since, limit))
        .await?;
    Ok(Json(batch).into_response())
}

async fn poke(
    State(node): State<Arc<Inner>>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<PokeBody>,
) -> StatusCode {
    let addr = join_host_port(&remote.ip().to_canonical().to_string(), body.port.into());
    node.learn(&addr);
    let node = node.clone();
    tokio::spawn(async move { node.pull_peer(&addr).await });
    StatusCode::NO_CONTENT
}

async fn claim(
    State(node): State<Arc<Inner>>,
    Json(claim): Json<NumberClaim>,
) -> Result<Response, ApiError> {
    let reply = node.blocking(move |db| db.claim_number(claim)).await?;
    Ok(Json(reply).into_response())
}

async fn snapshot(State(node): State<Arc<Inner>>) -> Result<Response, ApiError> {
    let dir = node
        .db
        .path()
        .parent()
        .map(|p| p.join("snapshots"))
        .unwrap_or_else(|| "snapshots".into());
    let tmp = dir.join(format!("{}.db", uuid::Uuid::new_v4()));
    let path = tmp.clone();
    let info = node.blocking(move |db| db.snapshot(path)).await?;
    let bytes = tokio::fs::read(&tmp).await;
    let _ = tokio::fs::remove_file(&tmp).await;
    let bytes = bytes.map_err(Error::from)?;
    tracing::info!(size = bytes.len(), head = info.head, "sent a join snapshot");

    let info = serde_json::to_string(&info).map_err(sanco_core::Error::from)?;
    let mut res = bytes.into_response();
    let headers = res.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        SNAPSHOT_HEADER,
        HeaderValue::from_str(&info).expect("JSON of ids and numbers is a valid header"),
    );
    Ok(res)
}

fn reject(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

struct ApiError(Error);

impl<E: Into<Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(e.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        tracing::warn!(error = %self.0, "sync request failed");
        reject(StatusCode::INTERNAL_SERVER_ERROR, &self.0.to_string())
    }
}

/// This version's installer, read fresh (it is small and rarely asked for).
async fn installer(node: &Inner) -> Result<Option<(update::UpdateInfo, Vec<u8>)>, ApiError> {
    let Some(path) = node.cfg.installer.clone() else {
        return Ok(None);
    };
    let version = node.cfg.app_version.clone();
    let read = tokio::task::spawn_blocking(move || {
        path.is_file()
            .then(|| update::describe(&path, &version))
            .transpose()
    })
    .await
    .map_err(|e| Error::Task(e.to_string()))?;
    Ok(read?)
}

async fn update_info(State(node): State<Arc<Inner>>) -> Result<Response, ApiError> {
    Ok(match installer(&node).await? {
        Some((info, _)) => Json(info).into_response(),
        None => reject(StatusCode::NOT_FOUND, "this PC has no installer to share"),
    })
}

async fn update_installer(State(node): State<Arc<Inner>>) -> Result<Response, ApiError> {
    let Some((info, bytes)) = installer(&node).await? else {
        return Ok(reject(
            StatusCode::NOT_FOUND,
            "this PC has no installer to share",
        ));
    };
    tracing::info!(version = %info.version, size = info.size, "sent the installer to a peer");
    let mut res = bytes.into_response();
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    Ok(res)
}
