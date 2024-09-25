use router_config_lib::AccountDataSourceConfig;
use services_mango_lib::postgres_configuration::PostgresConfiguration;

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct Config {
    pub source: AccountDataSourceConfig,
    pub metrics: MetricsConfig,
    pub postgres: PostgresConfiguration,
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct MetricsConfig {
    pub enabled: bool,
}
