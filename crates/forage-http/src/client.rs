//! Live `reqwest`-backed transport with rate limiting + retry.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use indexmap::IndexMap;
use reqwest::{Client, Method};
use tokio::sync::Mutex;
use tracing::warn;

use crate::error::{HttpError, HttpResult};
use crate::transport::{HttpRequest, HttpResponse, Transport};

#[derive(Debug, Clone)]
pub struct LiveTransportConfig {
    /// Minimum interval between successive requests (default 1.0s).
    pub min_interval: Duration,
    /// Max retries on 429 / 5xx (default 3).
    pub max_retries: u32,
    /// Initial backoff for retries (default 500 ms; doubles each retry).
    pub initial_backoff: Duration,
    /// Per-request timeout.
    pub request_timeout: Duration,
    /// Connect timeout.
    pub connect_timeout: Duration,
}

impl Default for LiveTransportConfig {
    fn default() -> Self {
        Self {
            min_interval: Duration::from_secs(1),
            max_retries: 3,
            initial_backoff: Duration::from_millis(500),
            request_timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
        }
    }
}

pub struct LiveTransport {
    client: Client,
    cfg: LiveTransportConfig,
    last_request: Arc<Mutex<Option<Instant>>>,
}

impl LiveTransport {
    pub fn new() -> HttpResult<Self> {
        Self::with_config(LiveTransportConfig::default())
    }

    pub fn with_config(cfg: LiveTransportConfig) -> HttpResult<Self> {
        let client = Client::builder()
            .cookie_store(true)
            .timeout(cfg.request_timeout)
            .connect_timeout(cfg.connect_timeout)
            .build()
            .map_err(|e| HttpError::Transport(format!("reqwest builder: {e}")))?;
        Ok(Self {
            client,
            cfg,
            last_request: Arc::new(Mutex::new(None)),
        })
    }

    async fn throttle(&self) {
        let mut last = self.last_request.lock().await;
        if let Some(t) = *last {
            let elapsed = t.elapsed();
            if elapsed < self.cfg.min_interval {
                tokio::time::sleep(self.cfg.min_interval - elapsed).await;
            }
        }
        *last = Some(Instant::now());
    }

    async fn send_once(&self, req: &HttpRequest) -> HttpResult<HttpResponse> {
        let method = Method::from_bytes(req.method.as_bytes())
            .map_err(|e| HttpError::Transport(format!("invalid method '{}': {e}", req.method)))?;
        let mut builder = self.client.request(method, &req.url);
        for (k, v) in &req.headers {
            builder = builder.header(k, v);
        }
        if let Some(body) = &req.body {
            builder = builder.body(body.clone());
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| HttpError::Transport(format!("send {}: {e}", req.url)))?;
        let status = resp.status().as_u16();
        let mut headers: IndexMap<String, String> = IndexMap::new();
        for (k, v) in resp.headers() {
            if let Ok(s) = v.to_str() {
                headers.insert(k.to_string(), s.to_string());
            }
        }
        let body = resp
            .bytes()
            .await
            .map_err(|e| HttpError::Transport(format!("read body: {e}")))?
            .to_vec();
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

#[async_trait]
impl Transport for LiveTransport {
    async fn fetch(&self, req: HttpRequest) -> HttpResult<HttpResponse> {
        let mut attempt: u32 = 0;
        let mut backoff = self.cfg.initial_backoff;
        loop {
            self.throttle().await;
            let resp = self.send_once(&req).await?;
            let retriable = resp.status == 429 || (500..600).contains(&resp.status);
            if !retriable || attempt >= self.cfg.max_retries {
                return Ok(resp);
            }
            attempt += 1;
            let retry_after = resp
                .headers
                .get("retry-after")
                .or_else(|| resp.headers.get("Retry-After"))
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);
            let wait = retry_after.unwrap_or(backoff);
            warn!(
                status = resp.status,
                attempt,
                wait_ms = wait.as_millis() as u64,
                "retrying"
            );
            tokio::time::sleep(wait).await;
            backoff *= 2;
        }
    }
}
