# Swap vs quote consistency tests

### Increasing max_map_count

As the testing code uses account db which requires lots of threads and access lots of files.
We have to increase the vm.max_map_count and vm.nr_open.
We will get 'out of memory exception' without this configuration change.

```
# in file /etc/sysctl.conf set / only to be done once per machine
vm.max_map_count=1000000
fs.nr_open = 1000000
```

To reload config :

```
sudo sysctl -p
```

### Capture for swap tests

```
export RPC_HTTP_URL="http://fcs-ams1._peer.internal:18899"
```

```
DUMP_MAINNET_DATA=1 RUST_LOG=info cargo test --package dex-orca -- --nocapture

DUMP_MAINNET_DATA=1 RUST_LOG=info cargo test --package dex-saber -- --nocapture

DUMP_MAINNET_DATA=1 RUST_LOG=info cargo test --package dex-raydium-cp -- --nocapture

DUMP_MAINNET_DATA=1 RUST_LOG=info cargo test --package dex-raydium -- --nocapture

DUMP_MAINNET_DATA=1 RUST_LOG=info cargo test --package dex-openbook-v2 -- --nocapture

DUMP_MAINNET_DATA=1 RUST_LOG=info cargo test --package dex-infinity -- --nocapture
```

---

### Run for every dex

```
cargo test-sbf --package simulator -- --nocapture
```

### Run for each dex one by one

```
cargo test-sbf --package simulator -- --nocapture cases::test_swap_from_dump::test_quote_match_swap_for_saber 
cargo test-sbf --package simulator -- --nocapture cases::test_swap_from_dump::test_quote_match_swap_for_orca 
cargo test-sbf --package simulator -- --nocapture cases::test_swap_from_dump::test_quote_match_swap_for_cropper 
cargo test-sbf --package simulator -- --nocapture cases::test_swap_from_dump::test_quote_match_swap_for_raydium_cp 
cargo test-sbf --package simulator -- --nocapture cases::test_swap_from_dump::test_quote_match_swap_for_gamma 
cargo test-sbf --package simulator -- --nocapture cases::test_swap_from_dump::test_quote_match_swap_for_raydium 
cargo test-sbf --package simulator -- --nocapture cases::test_swap_from_dump::test_quote_match_swap_for_openbook_v2 
cargo test-sbf --package simulator -- --nocapture cases::test_swap_from_dump::test_quote_match_swap_for_infinity 
```

---

# Run router

```
cargo build --bin router && RUST_LOG=info ./target/debug/router config/small.toml
```

---

# Perf test:

### Step 1 dump data

```
RUST_LOG=info RPC_HTTP_URL="http://fcs-ams1._peer.internal:18899" cargo test --package router --release -- dump_all_dex_data --nocapture
```

### Step 2 run perf test

```
RUST_LOG=info cargo test --package router --release -- path_finding_perf_test --nocapture

RUST_LOG=info cargo test --package router --release -- path_warmup_perf_test --nocapture
```