# Fill.city Autobahn

![logo](./brand/autobahn-logo-mark.svg)

Autobahn is the open source aggregator for swaps on Solana.
This public good protocol enables developers to contribute their own DEX adapters.
Take back control: access to orderflow from routers on Solana should not be centralized.

The graph search is optimized for reliability of trade execution.
Reliability is preferred over marginal price to improve user experience.
Full test coverage through daily verification of all routed pools ensures correctness.

A hosted version is available.
Reach out to partnerships@mango.markets to get an access token.
Self-hosting requires custom validator patches to enable low-latency account subscriptions.

## Using the router (as a client)

Basically it is the same API as Jupiter:
`https://autobahn.mngo.cloud/<TOKEN>/`

### quote (GET)

Supported parameters:
- inputMint
- outputMint
- amount
- slippageBps
- maxAccounts
- onlyDirectRoutes

### swap & swap-instructions (POST)

Supported parameters:

- userPublicKey
- wrapAndUnwrapSol
- autoCreateOutAta
- quoteResponse

## Running the router

See example configuration file [example-config.toml](bin/autobahn-router/example-config.toml) to create your own setup

Run like this:

```
RUST_LOG=info router my_config.toml
```

## Creating a new DEX Adapter

Adding new DEX adapter is welcome, you can do a pull-request, it will be appreciated !

See [CreatingAnAdapter.MD](CreatingAnAdapter.MD) file for details.

## Integration testing

It's possible to dump data from mainnet, and then use that in tests:
- To assert quoting is correct (same result as simulated swap)
- To check router path finding perfomance
 
See [Testing.MD](Testing.MD) file for details.

There's a script for daily smoke tests:

```
RPC_HTTP_URL=... ./scripts/smoke-test.sh
```

## Tokio-Console

Build router with feature `tokio-console` and `RUSTFLAGS="--cfg tokio_unstable"` like this:

```RUSTFLAGS="--cfg tokio_unstable" cargo build --bin router --release --features tokio-console```

And use the `tokio-console` crate to display running tasks

## Trigger automatic build and deployment to fly.io

```bash
git tag production/router-202409231509
git tag production/indexer-202409231509
git tag production/comparer-202409231509
# push tag(s)
```

## License

Autobahn is published under GNU Affero General Public License v3.0.
In case you are interested in an alternative license please reach out to partnerships@mango.markets
