use crate::env_helper::string_or_env;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, Default)]
pub struct PostgresConfiguration {
    #[serde(deserialize_with = "string_or_env")]
    pub connection_string: String,
    pub allow_invalid_certs: bool,
    pub tls: Option<PostgresTlsConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PostgresTlsConfig {
    /// CA Cert file or env var
    pub ca_cert_path: String,
    /// PKCS12 client cert path
    pub client_key_path: String,
}
