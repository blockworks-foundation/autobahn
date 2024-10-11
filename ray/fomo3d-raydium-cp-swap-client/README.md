re: multiswap

just use it for arbs

it won't work for much else

cargo run multiswap EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v 1000000

# Fomo3D Raydium CP Swap Client

This README provides instructions on how to set up and use the Fomo3D Raydium CP Swap Client.

## Configuration

To use this client, you need to create a `client_config.ini` file in your current directory with the following content:

[Global]

http_url = https://api.mainnet-beta.solana.com

ws_url = wss://api.mainnet-beta.solana.com

payer_path = /Users/jarettdunn/99.json

admin_path =/Users/jarettdunn/99.json

raydium_cp_program = 2Ak7T2tKpg1HgjcvbEc58mMKbSDbPbLumGGacyidpqtY

slippage = 0.5


## Features

### ELI5 (Explain Like I'm 5)

Imagine you have a special lemonade stand. When people help you by giving you lemons and sugar, you give them special tickets. The more they help, the harder it gets to earn tickets. When they trade lemonade, you keep a tiny bit for yourself and your helpers. Your stand is special because it has a fun name tag that you can change!

### ELI18 (Explain Like I'm 18)

This AMM (Automated Market Maker) on Solana has some unique features:

1. LP Token Curve: It uses an exponential bonding curve for minting and burning LP tokens. This means as more liquidity is added, it becomes progressively harder to mint new LP tokens, potentially rewarding early liquidity providers more.

2. Flat-rate Fees: Unlike percentage-based fees in most AMMs, this uses flat-rate fees. This benefits larger trades as they're less impacted by fees proportionally.

3. Fee Distribution: Fees are split between liquidity providers and the protocol creator indefinitely, creating a sustainable revenue model for the protocol.

4. Customizable LP Tokens: The system allows for customizable metadata for LP tokens, potentially improving their utility and recognition in the broader DeFi ecosystem.

5. Constant Token Prices: Despite the exponential LP token curve, the actual token prices in the pool remain based on the constant product formula, maintaining familiar swap dynamics.

### ELIVitalik (Explain Like I'm Vitalik Buterin)

This Solana-based AMM protocol introduces several innovative mechanisms that diverge from traditional constant product markets:

1. Exponential Bonding Curve for LP Tokens: The protocol implements an exponential bonding curve for minting and burning LP tokens, while maintaining a constant product formula for token pricing. This creates an interesting dynamic where the marginal cost of liquidity provision increases over time, potentially incentivizing early liquidity and creating a form of "liquidity moat". This could lead to improved capital efficiency but may also introduce new game-theoretic considerations for liquidity providers.

2. Flat-rate Fee Structure: The adoption of flat-rate fees, as opposed to the typical percentage-based fees, presents an intriguing economic model. This structure inherently favors larger trades by reducing their proportional cost. While this could attract more substantial liquidity and potentially reduce the impact of MEV, it may also affect the protocol's ability to capture value from high-value transactions. The implications on pool balance and impermanent loss under various market conditions would be an interesting area for further analysis.

3. Perpetual Creator Fees: The indefinite fee allocation to the protocol creator introduces a novel sustainability model. While this ensures ongoing development incentives, it also raises questions about long-term incentive alignment and potential regulatory implications, especially in jurisdictions that might view such structures as akin to equity-like returns.

4. Customizable LP Token Metadata: This feature enhances the composability of LP tokens within the broader DeFi ecosystem. It could potentially facilitate more complex financial products built on top of these LP positions, although it also introduces questions about standardization and interoperability.

5. Solana Integration: The implementation on Solana leverages the blockchain's account model and high-throughput capabilities. This allows for potentially more complex on-chain logic and faster execution, but also introduces different security considerations compared to Ethereum-based AMMs, particularly around transaction ordering and MEV mitigation.

The combination of these features creates a unique market microstructure that warrants deeper analysis, particularly in terms of its long-term equilibrium properties, resilience against various attack vectors, and overall market efficiency in comparison to traditional constant product markets.

https://x.com/STACCoverflow/status/1832166465774227765
