CREATE SCHEMA IF NOT EXISTS router AUTHORIZATION CURRENT_ROLE;

CREATE TABLE IF NOT EXISTS router.comparison
(
    timestamp TIMESTAMP WITH TIME ZONE NOT NULL,
    input_mint VARCHAR(64) NOT NULL,
    output_mint VARCHAR(64) NOT NULL,
    input_amount bigint NOT NULL,
    router_quote_output_amount bigint NOT NULL,
    jupiter_quote_output_amount bigint NOT NULL,
    router_simulation_success BOOLEAN NOT NULL,
    jupiter_simulation_success BOOLEAN NOT NULL,
    max_accounts bigint NOT NULL,
    router_accounts bigint NOT NULL,
    jupiter_accounts bigint NOT NULL,
    input_amount_in_dollars double precision NOT NULL,
    router_output_amount_in_dollars double precision NOT NULL,
    jupiter_output_amount_in_dollars double precision NOT NULL
)

grant select, insert on router.comparison to router_indexer;

ALTER TABLE router.comparison ADD router_actual_output_amount bigint;
ALTER TABLE router.comparison ADD jupiter_actual_output_amount bigint;
ALTER TABLE router.comparison ADD router_error TEXT;
ALTER TABLE router.comparison ADD jupiter_error TEXT;