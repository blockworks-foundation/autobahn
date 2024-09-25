use solana_sdk::signature::Keypair;
use solana_sdk::signer::keypair;
use std::str::FromStr;

pub fn tracing_subscriber_init() {
    let format = tracing_subscriber::fmt::format().with_ansi(atty::is(atty::Stream::Stdout));

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .event_format(format)
        .init();
}

pub fn keypair_from_cli(keypair: &str) -> Keypair {
    let maybe_keypair = keypair::read_keypair(&mut keypair.as_bytes());
    match maybe_keypair {
        Ok(keypair) => keypair,
        Err(_) => {
            let path = std::path::PathBuf::from_str(&shellexpand::tilde(keypair)).unwrap();
            keypair::read_keypair_file(path)
                .unwrap_or_else(|_| panic!("Failed to read keypair from {}", keypair))
        }
    }
}
