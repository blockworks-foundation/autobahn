use std::collections::HashSet;

use mango_feeds_connector::SlotUpdate;
use solana_program::pubkey::Pubkey;

use router_config_lib::AccountDataSourceConfig;
use router_feed_lib::account_write::AccountOrSnapshotUpdate;
use router_feed_lib::get_program_account::FeedMetadata;

use crate::source::grpc_plugin_source;

use super::quic_plugin_source;

pub async fn spawn_geyser_source(
    config: &AccountDataSourceConfig,
    exit_receiver: tokio::sync::broadcast::Receiver<()>,
    account_write_sender: async_channel::Sender<AccountOrSnapshotUpdate>,
    metadata_write_sender: async_channel::Sender<FeedMetadata>,
    slot_sender: async_channel::Sender<SlotUpdate>,
    subscribed_accounts: &HashSet<Pubkey>,
    subscribed_programs: &HashSet<Pubkey>,
    subscribed_token_accounts: &HashSet<Pubkey>,
    filters: &HashSet<Pubkey>,
) {
    if config.quic_sources.is_some() {
        quic_plugin_source::process_events(
            config.clone(),
            subscribed_accounts.clone(),
            subscribed_programs.clone(),
            subscribed_token_accounts.clone(),
            filters.clone(),
            account_write_sender,
            Some(metadata_write_sender),
            slot_sender,
            exit_receiver,
        )
        .await;
    } else if config.grpc_sources.is_some() {
        grpc_plugin_source::process_events(
            config.clone(),
            subscribed_accounts.clone(),
            subscribed_programs.clone(),
            subscribed_token_accounts.clone(),
            filters.clone(),
            account_write_sender,
            Some(metadata_write_sender),
            slot_sender,
            exit_receiver,
        )
        .await;
    }
}
