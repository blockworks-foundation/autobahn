use crate::postgres_configuration::PostgresConfiguration;
use native_tls::{Certificate, Identity, TlsConnector};
use postgres_native_tls::MakeTlsConnector;
use std::{env, fs};
use tokio::task::JoinHandle;
use tokio_postgres::Client;

pub async fn connect(
    config: &PostgresConfiguration,
) -> anyhow::Result<(Client, JoinHandle<Result<(), tokio_postgres::Error>>)> {
    // openssl pkcs12 -export -in client.cer -inkey client-key.cer -out client.pks
    // base64 -i ca.cer -o ca.cer.b64 && base64 -i client.pks -o client.pks.b64
    // fly secrets set PG_CA_CERT=- < ./ca.cer.b64 -a mango-fills
    // fly secrets set PG_CLIENT_KEY=- < ./client.pks.b64 -a mango-fills
    let tls = match &config.tls {
        Some(tls) => {
            use base64::{engine::general_purpose, Engine as _};
            let ca_cert = match &tls.ca_cert_path.chars().next().unwrap() {
                '$' => general_purpose::STANDARD
                    .decode(
                        env::var(&tls.ca_cert_path[1..])
                            .expect("reading client cert from env")
                            .into_bytes(),
                    )
                    .expect("decoding client cert"),
                _ => fs::read(&tls.ca_cert_path).expect("reading client cert from file"),
            };
            let client_key = match &tls.client_key_path.chars().next().unwrap() {
                '$' => general_purpose::STANDARD
                    .decode(
                        env::var(&tls.client_key_path[1..])
                            .expect("reading client key from env")
                            .into_bytes(),
                    )
                    .expect("decoding client key"),
                _ => fs::read(&tls.client_key_path).expect("reading client key from file"),
            };
            MakeTlsConnector::new(
                TlsConnector::builder()
                    .add_root_certificate(Certificate::from_pem(&ca_cert)?)
                    .identity(Identity::from_pkcs12(&client_key, "pass")?)
                    .danger_accept_invalid_certs(config.allow_invalid_certs)
                    .build()?,
            )
        }
        None => MakeTlsConnector::new(
            TlsConnector::builder()
                .danger_accept_invalid_certs(config.allow_invalid_certs)
                .build()?,
        ),
    };

    let config = config.clone();

    let (client, connection) = tokio_postgres::connect(&config.connection_string, tls).await?;

    let handle = tokio::spawn(async move { connection.await });

    Ok((client, handle))
}
