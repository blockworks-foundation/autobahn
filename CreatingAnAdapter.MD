# Creating a Dex Adapter

## Architecture Summary

Router will call initialize on every Dex Adapter to gather a list of edge and account/program subscriptions for live refresh.

Dex must also provide a map of key to edge(s) so that we know what quote should be refreshed when some accounts are modified on chain. 

> Edge = One way (pool/market) to exchange an input token for an output token

## Implementation

Please create a new library inside the `lib` folder named like this: `dex-<your-name>` 

You will need to implement three traits:

- DexInterface
- DexEdgeIdentifier
- DexEdge 

Looking at existing implementation (raydium for example) is recommended to start. 

### DexInterface

Trait used to:

- Load pools / list accounts for which a change should trigger a repricing
- Compute quote
- Generate swap IX

### DexEdgeIdentifier

Static data used to identify an edge (pool/market+direction)

- `key()` 
- `input_mint()`
- `output_mint()`
- `accounts_needed()` 
 this should count all account needed to make a swap, excluding:
  - user wallet address
  - user output ATA

Long living object.

**Warning**: key + input_mint should be unique  

### DexEdge

Accounts data needed to compute a quote for a given edge.

Short living object, loaded by Autobahn when updating prices and finding best path.

## Testing and validating

Also see [Testing.MD](Testing.MD)

Please create a test directory in your dex adapter lib with, at least, one end-to-end test.

A three steps design is preferred:

- 1/ Capture all accounts needed 
- 2/ Generate at least one swap for every edge
- 3/ Simulate execution

You can basically copy and paste the `test_raydium.rs` for a simple example of 1 and 2. 
For 3, you need to update `test_swap_from_dump.rs` in the simulator test program (again, take inspiration from raydium implementation: `test_quote_match_swap_for_raydium`). 

Running should be done like this:

```
DUMP_MAINNET_DATA=1 RUST_LOG=info RPC_HTTP_URL="..." cargo test --package dex-<your-name>
cargo test-sbf --package simulator -- --nocapture cases::test_swap_from_dump::test_quote_match_swap_for_<your-name>
```

Note the test runner automatically captures accounts that were not
specified in the dex adapters subscription_mode fn. This is based on
observation of the generated instructions and ensures all accounts
used even the ones not used to calculate a quote are included in the
snapshot.
This can cause data hazards bc. the snapshot is not generated at a
consistent blockheight, it's recommended to update the subscription
mode fn in this case.
