#!/bin/bash
# Test the integration of the application contract and publisher, against a running EVM chain.
set -e -o pipefail

# Determine the chain ID
CHAIN_ID=$(cast rpc --rpc-url ${ETH_RPC_URL:?} eth_chainId | jq -re)
CHAIN_ID=$((CHAIN_ID))

# Extract the block in which the Toyken contract has been deployed
# BLOCK_NUMBER=$(jq --arg ADDRESS "$TOYKEN_ADDRESS" -re '.receipts[] | select(.contractAddress == $ADDRESS) | .blockNumber' ./broadcast/DeployCounter.s.sol/$CHAIN_ID/run-latest.json)
BLOCK_NUMBER=$(cast block-number --rpc-url ${ETH_RPC_URL} | jq -re | xargs cast to-hex)
export COMMITMENT_BLOCK=$BLOCK_NUMBER

# Enable the history feature and override the commitment block
if [[ ${HISTORY_BLOCKS} -gt 0 ]]; then
  printf -v COMMITMENT_BLOCK '%#x' "$((BLOCK_NUMBER + HISTORY_BLOCKS))"
  PUBLISHER_FEATURES="history"
fi

echo "Waiting for block ${COMMITMENT_BLOCK} to have one confirmation..."
# while [[ $(cast rpc --rpc-url "${ETH_RPC_URL:?}" eth_blockNumber | jq -re) -le ${COMMITMENT_BLOCK} ]]; do sleep 3; done

# Publish a new state
echo "Publishing a new state..."
RISC0_DEV_MODE=true RISC0_INFO=1 RUST_LOG=${RUST_LOG:-info,risc0_steel=debug} cargo run --bin compound_apr_publisher -F "$PUBLISHER_FEATURES" -- \
  --eth-wallet-private-key=${ETH_WALLET_PRIVATE_KEY:?} \
  --eth-rpc-url=${ETH_RPC_URL:?} \
  --execution-block=${BLOCK_NUMBER:?}

# Attempt to verify counter value as part of the script logic
echo "Verifying state..."
