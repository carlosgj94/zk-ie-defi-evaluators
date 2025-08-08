#!/bin/bash
# Test the integration of the application contract and publisher, against a running EVM chain.
set -e -o pipefail

export TOKEN_OWNER=${ETH_WALLET_ADDRESS:?}
export TOKEN_OWNER_2=${TOKEN_OWNER_2:?}

# Determine the chain ID
CHAIN_ID=$(cast rpc --rpc-url ${ETH_RPC_URL:?} eth_chainId | jq -re)
CHAIN_ID=$((CHAIN_ID))

# Extract the Token address
echo "ERC20 Token Address: $TOKEN_ADDRESS"

BLOCK_NUMBER=$(cast block-number --rpc-url ${ETH_RPC_URL} | jq -re | xargs cast to-hex)
export COMMITMENT_BLOCK=$BLOCK_NUMBER

export PAST_BLOCK_NUMBER=${PAST_BLOCK_NUMBER:?}

# Enable the history feature and override the commitment block
if [[ ${HISTORY_BLOCKS} -gt 0 ]]; then
  printf -v COMMITMENT_BLOCK '%#x' "$((BLOCK_NUMBER + HISTORY_BLOCKS))"
  PUBLISHER_FEATURES="history"
fi

# Publish a new state
echo "Publishing a new state..."
RISC0_INFO=1 RUST_LOG=${RUST_LOG:-info,risc0_steel=debug} cargo run --bin publisher -F "$PUBLISHER_FEATURES" -- \
  --eth-wallet-private-key=${ETH_WALLET_PRIVATE_KEY:?} \
  --eth-rpc-url=${ETH_RPC_URL:?} \
  --execution-block=${BLOCK_NUMBER:?} \
  --past-execution-block=${PAST_BLOCK_NUMBER:?} \
  --token-contract=${TOKEN_ADDRESS:?} \
  --account=${TOKEN_OWNER:?} \
  --account-2=${TOKEN_OWNER_2:?}
