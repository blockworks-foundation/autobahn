use axum::{routing, Router};
use prometheus::{Encoder, TextEncoder};
use tokio::net::{TcpListener, ToSocketAddrs};
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::server::errors::AppError;

pub struct PrometheusSync;

impl PrometheusSync {
    fn create_response(payload: String) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            payload.len(),
            payload
        )
    }

    async fn get_prometheus_stream() -> Result<String, AppError> {
        let mut metrics_buffer = Vec::new();
        let encoder = TextEncoder::new();

        let metric_families = prometheus::gather();
        encoder
            .encode(&metric_families, &mut metrics_buffer)
            .unwrap();

        let metrics_buffer = String::from_utf8(metrics_buffer).unwrap();
        Ok(Self::create_response(metrics_buffer))
    }

    pub fn sync(addr: impl ToSocketAddrs + Send + 'static) -> JoinHandle<anyhow::Result<()>> {
        tokio::spawn(async move {
            let listener = TcpListener::bind(addr).await?;

            let mut router: Router<()> = Router::new();
            router = router.route("/metrics", routing::get(Self::get_prometheus_stream));

            let handle = axum::serve(listener, router);

            info!("Prometheus Server started");

            handle.await.expect("Prometheus Server failed");
            Ok(())
        })
    }
}
