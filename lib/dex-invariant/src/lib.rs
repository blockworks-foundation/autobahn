mod internal;
mod invariant_dex;
mod invariant_edge;
mod invariant_ix_builder;

pub use invariant_dex::InvariantDex;

use solana_sdk::declare_id;

// declare_id!("HyaB3W9q6XdA5xwpU4XnSZV94htfmbmqJXZcEbRaJutt");
declare_id!("D8Xd5VFXJeANivc4LXEzYqiE8q2CGVbjym5JiynPCP6J");

mod authority {
    use solana_sdk::declare_id;

    declare_id!("5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1");
}
