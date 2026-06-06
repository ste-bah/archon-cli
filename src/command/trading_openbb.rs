use anyhow::Result;
use archon_trading::adapters::openbb::{
    AccessMode, DataQuality, OpenBbError, OpenBbGateway, OpenBbRequest, OpenBbResponse,
    OpenBbTransport,
};
use archon_trading::data_store::{StoreOhlcvRequest, TradingDataLake};
use archon_trading::ohlcv::{OhlcvFormat, parse_ohlcv};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::cli_args::{TradingCliOhlcvFormat, TradingCliOpenBbAction, TradingCliOpenBbMode};
use crate::command::trading_io::{read_json, write_or_render};

use super::trading_tools::{openbb_bin, openbb_venv_dir, project_root};

pub(crate) fn render_openbb(action: &TradingCliOpenBbAction) -> Result<String> {
    match action {
        TradingCliOpenBbAction::Status { target } => render_status(target.as_ref()),
        TradingCliOpenBbAction::Fetch {
            request,
            metadata,
            quality,
            mode,
            target,
            out,
            store_ohlcv,
            response_format,
        } => fetch(
            request,
            metadata,
            quality,
            *mode,
            target.as_ref(),
            out.as_deref(),
            *store_ohlcv,
            *response_format,
        ),
    }
}

fn render_status(target: Option<&PathBuf>) -> Result<String> {
    let root = project_root(target)?;
    let api_url =
        std::env::var("OPENBB_API_URL").unwrap_or_else(|_| "http://127.0.0.1:6900".into());
    let api_bin = openbb_bin(&root, "openbb-api");
    let py_bin = openbb_bin(&root, "python");
    let venv = openbb_venv_dir(&root);
    Ok([
        "OpenBB runtime".to_string(),
        format!("  project: {}", root.display()),
        format!("  venv: {}", path_state(&venv)),
        format!("  python: {}", path_state(&py_bin)),
        format!("  openbb-api: {}", path_state(&api_bin)),
        format!("  OPENBB_API_URL: {api_url}"),
        "".into(),
        "Start local API when installed:".into(),
        format!("  {} --host 127.0.0.1 --port 6900", api_bin.display()),
    ]
    .join("\n"))
}

fn path_state(path: &std::path::Path) -> String {
    if path.exists() {
        format!("present ({})", path.display())
    } else {
        format!("missing ({})", path.display())
    }
}

fn fetch(
    request_path: &std::path::Path,
    metadata_path: &std::path::Path,
    quality_path: &std::path::Path,
    mode: TradingCliOpenBbMode,
    target: Option<&PathBuf>,
    out: Option<&std::path::Path>,
    store_ohlcv: bool,
    response_format: TradingCliOhlcvFormat,
) -> Result<String> {
    let root = project_root(target)?;
    let request: OpenBbRequest = read_json(request_path, "OpenBbRequest")?;
    let metadata: BTreeMap<String, String> = read_json(metadata_path, "OpenBB metadata")?;
    let quality: DataQuality = read_json(quality_path, "DataQuality")?;
    let base_url =
        std::env::var("OPENBB_API_URL").unwrap_or_else(|_| "http://127.0.0.1:6900".into());
    let mut transport = LocalOpenBbApiTransport {
        base_url,
        metadata,
        quality,
    };
    let mut gateway = OpenBbGateway::default();
    let fetched_at = timestamp();
    let dataset = gateway
        .fetch(&mut transport, request, mode.into(), fetched_at.clone())
        .map_err(|error| anyhow::anyhow!("OpenBB fetch failed: {}", error.code()))?;
    let stored_ohlcv = if store_ohlcv {
        Some(store_openbb_ohlcv(
            &root,
            &dataset,
            response_format,
            fetched_at,
        )?)
    } else {
        None
    };
    let report = json!({
        "accepted": true,
        "project": root.display().to_string(),
        "dataset": dataset,
        "stored_ohlcv": stored_ohlcv,
        "provenance_log": gateway.provenance_log(),
        "lake_datasets": gateway.lake_registry().all().collect::<Vec<_>>()
    });
    write_or_render(&report, out)
}

fn store_openbb_ohlcv(
    root: &std::path::Path,
    dataset: &archon_trading::adapters::openbb::GovernedDataset,
    response_format: TradingCliOhlcvFormat,
    created_at: String,
) -> Result<archon_trading::data_store::StoredDatasetRecord> {
    let format = ohlcv_format(response_format);
    let bars = parse_ohlcv(&dataset.body, format)
        .map_err(|err| anyhow::anyhow!("OpenBB OHLCV response was not parseable: {err:?}"))?;
    TradingDataLake::new(root)
        .store_ohlcv(StoreOhlcvRequest {
            metadata: dataset.metadata.clone(),
            bars,
            raw_body: dataset.body.clone(),
            raw_format: format,
            created_at,
        })
        .map_err(|err| anyhow::anyhow!("failed to store OpenBB OHLCV dataset: {err:?}"))
}

struct LocalOpenBbApiTransport {
    base_url: String,
    metadata: BTreeMap<String, String>,
    quality: DataQuality,
}

impl OpenBbTransport for LocalOpenBbApiTransport {
    fn discover(&self, _request: &OpenBbRequest) -> bool {
        true
    }

    fn rest(&mut self, request: &OpenBbRequest) -> Result<OpenBbResponse, OpenBbError> {
        let url = self.url(request)?;
        let response = reqwest::blocking::Client::new()
            .get(url)
            .query(&request.params)
            .send()
            .map_err(|_| OpenBbError::TransportUnavailable)?;
        if response.status().as_u16() == 429 {
            return Err(OpenBbError::RateLimited);
        }
        if !response.status().is_success() {
            return Err(OpenBbError::TransportUnavailable);
        }
        let bytes = response
            .bytes()
            .map_err(|_| OpenBbError::TransportUnavailable)?
            .to_vec();
        Ok(OpenBbResponse {
            body: bytes,
            warnings: Vec::new(),
            metadata: self.metadata.clone(),
            quality: self.quality.clone(),
        })
    }

    fn sdk(&mut self, _request: &OpenBbRequest) -> Result<OpenBbResponse, OpenBbError> {
        Err(OpenBbError::TransportUnavailable)
    }

    fn mcp(&mut self, _request: &OpenBbRequest) -> Result<OpenBbResponse, OpenBbError> {
        Err(OpenBbError::TransportUnavailable)
    }
}

impl LocalOpenBbApiTransport {
    fn url(&self, request: &OpenBbRequest) -> Result<reqwest::Url, OpenBbError> {
        let base = self.base_url.trim_end_matches('/');
        let endpoint = request.endpoint.trim_start_matches('/');
        reqwest::Url::parse(&format!("{base}/{endpoint}"))
            .map_err(|_| OpenBbError::TransportUnavailable)
    }
}

impl From<TradingCliOpenBbMode> for AccessMode {
    fn from(value: TradingCliOpenBbMode) -> Self {
        match value {
            TradingCliOpenBbMode::Research => Self::Research,
            TradingCliOpenBbMode::LiveRequired => Self::LiveRequired,
        }
    }
}

fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn ohlcv_format(value: TradingCliOhlcvFormat) -> OhlcvFormat {
    match value {
        TradingCliOhlcvFormat::Csv => OhlcvFormat::Csv,
        TradingCliOhlcvFormat::Json => OhlcvFormat::Json,
    }
}
