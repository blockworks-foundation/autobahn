use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug, Clone)]
#[clap()]
pub struct Cli {
    #[clap(subcommand)]
    pub command: Command,
}

#[derive(Args, Debug, Clone)]
pub struct Rpc {
    #[clap(short, long, default_value = "m")]
    pub url: String,

    #[clap(short, long, default_value = "")]
    pub fee_payer: String,
}

#[derive(Args, Debug, Clone)]
pub struct Quote {
    #[clap(long)]
    pub input_mint: String,

    #[clap(long)]
    pub output_mint: String,

    #[clap(short, long)]
    pub amount: u64,

    #[clap(short, long, default_value = "50")]
    pub slippage_bps: u64,

    #[clap(short, long, default_value = "http://localhost:8888")]
    pub router: String,

    // can be either ExactIn or ExactOut
    #[clap(short, long, default_value = "ExactIn")]
    pub swap_mode: String,
}

#[derive(Args, Debug, Clone)]
pub struct Swap {
    #[clap(short, long)]
    pub owner: String,

    #[clap(long)]
    pub input_mint: String,

    #[clap(long)]
    pub output_mint: String,

    #[clap(short, long)]
    pub amount: u64,

    #[clap(short, long, default_value = "50")]
    pub slippage_bps: u64,

    #[clap(short, long, default_value = "http://localhost:8888")]
    pub router: String,

    #[clap(flatten)]
    pub rpc: Rpc,

    // can be either ExactIn or ExactOut
    #[clap(short, long, default_value = "ExactIn")]
    pub swap_mode: String,
}

#[derive(Args, Debug, Clone)]
pub struct DownloadTestPrograms {
    #[clap(short, long)]
    pub config: String,

    #[clap(flatten)]
    pub rpc: Rpc,
}

#[derive(Args, Debug, Clone)]
pub struct DecodeLog {
    #[clap(short, long)]
    pub data: String,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    Swap(Swap),
    Quote(Quote),
    DownloadTestPrograms(DownloadTestPrograms),
    DecodeLog(DecodeLog),
}
