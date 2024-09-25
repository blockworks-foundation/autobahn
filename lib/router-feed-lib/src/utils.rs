use router_config_lib::TlsConfig;
use std::env;
use tracing::info;
use tracing_subscriber::fmt::format::FmtSpan;
use yellowstone_grpc_proto::tonic::transport::{Certificate, ClientTlsConfig, Identity};

pub fn make_tls_config(config: &TlsConfig) -> ClientTlsConfig {
    let server_root_ca_cert = match &config.ca_cert_path.chars().next().unwrap() {
        '$' => env::var(&config.ca_cert_path[1..])
            .expect("reading server root ca cert from env")
            .into_bytes(),
        _ => std::fs::read(&config.ca_cert_path).expect("reading server root ca cert from file"),
    };
    let server_root_ca_cert = Certificate::from_pem(server_root_ca_cert);
    let client_cert = match &config.client_cert_path.chars().next().unwrap() {
        '$' => env::var(&config.client_cert_path[1..])
            .expect("reading client cert from env")
            .into_bytes(),
        _ => std::fs::read(&config.client_cert_path).expect("reading client cert from file"),
    };
    let client_key = match &config.client_key_path.chars().next().unwrap() {
        '$' => env::var(&config.client_key_path[1..])
            .expect("reading client key from env")
            .into_bytes(),
        _ => std::fs::read(&config.client_key_path).expect("reading client key from file"),
    };
    let client_identity = Identity::from_pem(client_cert, client_key);
    let domain_name = match &config.domain_name.chars().next().unwrap() {
        '$' => env::var(&config.domain_name[1..]).expect("reading domain name from env"),
        _ => config.domain_name.clone(),
    };
    ClientTlsConfig::new()
        .ca_certificate(server_root_ca_cert)
        .identity(client_identity)
        .domain_name(domain_name)
}

pub fn tracing_subscriber_init() {
    let format = tracing_subscriber::fmt::format().with_ansi(atty::is(atty::Stream::Stdout));

    #[cfg(feature = "tokio-console")]
    console_subscriber::init();

    #[cfg(feature = "tokio-console")]
    info!("Enabling Tokio-console support");

    #[cfg(not(feature = "tokio-console"))]
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_span_events(FmtSpan::CLOSE)
        .event_format(format)
        .init();

    #[cfg(not(feature = "tokio-console"))]
    info!("No Tokio-console support");
}
