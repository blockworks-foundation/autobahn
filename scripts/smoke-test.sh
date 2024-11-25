#!/bin/bash

# pad string to the left with spaces: $(pad "foobar" 15)
function pad () { [ "$#" -gt 1 ] && [ -n "$2" ] && printf "%$2.${2#-}s" "$1"; }

# env settings
export DUMP_MAINNET_DATA=1 RUST_LOG=info
export RPC_HTTP_URL="http://fcs-da1._peer.internal:18899"  
# define in addition
# RPC_HTTP_URL="http://fcs-ams1._peer.internal:18899" 
# for eclipse
# export ECLIPSE=true
# export DISABLE_COMRPESSED_GPA=true

# saber
DUMP_SABER_START=$(date)
START=$(date +%s)
cargo test --package dex-saber -- --nocapture
DUMP_SABER_RC=$?
DUMP_SABER_RT=$(expr $(date +%s) - $START)

SIM_SABER_START=$(date)
START=date +%s
cargo test-sbf --package simulator -- --nocapture --exact cases::test_swap_from_dump::test_quote_match_swap_for_saber 
SIM_SABER_RC=$?
SIM_SABER_RT=$(expr `date +%s` - $START)

# openbook_v2
DUMP_OPENBOOK_V2_START=$(date)
START=$(date +%s)
 cargo test --package dex-openbook-v2 -- --nocapture
DUMP_OPENBOOK_V2_RC=$?
DUMP_OPENBOOK_V2_RT=$(expr $(date +%s) - $START)

SIM_OPENBOOK_V2_START=$(date)
START=$(date +%s)
cargo test-sbf --package simulator -- --nocapture --exact cases::test_swap_from_dump::test_quote_match_swap_for_openbook_v2
SIM_OPENBOOK_V2_RC=$?
SIM_OPENBOOK_V2_RT=$(expr $(date +%s) - $START)

# infinity
DUMP_INFINITY_START=$(date)
START=$(date +%s)
cargo test --package dex-infinity -- --nocapture
DUMP_INFINITY_RC=$?
DUMP_INFINITY_RT=$(expr $(date +%s) - $START)

SIM_INFINITY_START=$(date)
START=$(date +%s)
cargo test-sbf --package simulator -- --nocapture --exact cases::test_swap_from_dump::test_quote_match_swap_for_infinity
SIM_INFINITY_RC=$?
SIM_INFINITY_RT=$(expr $(date +%s) - $START)

# raydium cp
DUMP_RAYDIUM_CP_START=$(date)
START=$(date +%s)
cargo test --package dex-raydium-cp -- --nocapture
DUMP_RAYDIUM_CP_RC=$?
DUMP_RAYDIUM_CP_RT=$(expr $(date +%s) - $START)

SIM_RAYDIUM_CP_START=$(date)
START=$(date +%s)
cargo test-sbf --package simulator -- --nocapture --exact cases::test_swap_from_dump::test_quote_match_swap_for_raydium_cp
SIM_RAYDIUM_CP_RC=$?
SIM_RAYDIUM_CP_RT=$(expr $(date +%s) - $START)

# raydium
DUMP_RAYDIUM_START=$(date)
START=$(date +%s)
cargo test --package dex-raydium -- --nocapture
DUMP_RAYDIUM_RC=$?
DUMP_RAYDIUM_RT=$(expr $(date +%s) - $START)

SIM_RAYDIUM_START=$(date)
START=$(date +%s)
cargo test-sbf --package simulator -- --nocapture --exact cases::test_swap_from_dump::test_quote_match_swap_for_raydium
SIM_RAYDIUM_RC=$?
SIM_RAYDIUM_RT=$(expr $(date +%s) - $START)

# orca
DUMP_ORCA_START=$(date)
START=$(date +%s)
cargo test --package dex-orca -- --nocapture
DUMP_ORCA_RC=$?
DUMP_ORCA_RT=$(expr $(date +%s) - $START)

SIM_ORCA_START=$(date)
START=$(date +%s)
cargo test-sbf --package simulator -- --nocapture --exact cases::test_swap_from_dump::test_quote_match_swap_for_orca
SIM_ORCA_RC=$?
SIM_ORCA_RT=$(expr $(date +%s) - $START)


read -r -d '\0' FORMATTED <<- EOM
dex         stage   return-code   run-time   start-time
saber       dump    `pad $DUMP_SABER_RC 11`   `pad $DUMP_SABER_RT 8`   $DUMP_SABER_START
saber       sim     `pad $SIM_SABER_RC 11`   `pad $SIM_SABER_RT 8`   $SIM_SABER_START
openbook-v2 dump    `pad $DUMP_OPENBOOK_V2_RC 11`   `pad $DUMP_OPENBOOK_V2_RT 8`   $DUMP_OPENBOOK_V2_START
openbook-v2 sim     `pad $SIM_OPENBOOK_V2_RC 11`   `pad $SIM_OPENBOOK_V2_RT 8`   $SIM_OPENBOOK_V2_START
infinity    dump    `pad $DUMP_INFINITY_RC 11`   `pad $DUMP_INFINITY_RT 8`   $DUMP_INFINITY_START
infinity    sim     `pad $SIM_INFINITY_RC 11`   `pad $SIM_INFINITY_RT 8`   $SIM_INFINITY_START
raydium-cp  dump    `pad $DUMP_RAYDIUM_CP_RC 11`   `pad $DUMP_RAYDIUM_CP_RT 8`   $DUMP_RAYDIUM_CP_START
raydium-cp  sim     `pad $SIM_RAYDIUM_CP_RC 11`   `pad $SIM_RAYDIUM_CP_RT 8`   $SIM_RAYDIUM_CP_START
raydium     dump    `pad $DUMP_RAYDIUM_RC 11`   `pad $DUMP_RAYDIUM_RT 8`   $DUMP_RAYDIUM_START
raydium     sim     `pad $SIM_RAYDIUM_RC 11`   `pad $SIM_RAYDIUM_RT 8`   $SIM_RAYDIUM_START
orca        dump    `pad $DUMP_ORCA_RC 11`   `pad $DUMP_ORCA_RT 8`   $DUMP_ORCA_START
orca        sim     `pad $SIM_ORCA_RC 11`   `pad $SIM_ORCA_RT 8`   $SIM_ORCA_START
\0
EOM

echo "Autobahn smoke-test results:"
echo "$FORMATTED"