# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

This is a RISC Zero zkVM project that creates zero-knowledge proofs for economic data using RISC Zero and Steel. The project generates cryptographic proofs for financial metrics and economic data, demonstrating secure off-chain computation with on-chain verification through Ethereum smart contracts.

## Development Commands

### Quick Start
```bash
# Run the complete end-to-end test
./e2e-compound.sh
```

### Build Commands
```bash
# Build all Rust components (apps and methods)
cargo build

# Build Solidity contracts
forge build

# For reproducible builds using Docker
RISC0_USE_DOCKER=1 cargo build
```

### Test Commands
```bash
# Run Solidity tests
forge test

# Run a specific test
forge test --match-test testName

# Run Rust tests
cargo test

# Run end-to-end integration test (requires ETH_WALLET_ADDRESS, ETH_WALLET_PRIVATE_KEY, ETH_RPC_URL)
./e2e-compound.sh
```

### Linting
```bash
# Solidity linting
solhint 'contracts/**/*.sol'

# Rust linting
cargo clippy

# Rust formatting
cargo fmt
```

### Local Development
```bash
# Start local testnet
anvil

# Deploy contracts locally
forge script --rpc-url http://localhost:8545 --broadcast Deploy

# Run compound APR publisher (generates and submits economic data proofs)
RUST_LOG=info cargo run --bin compound_apr_publisher -- \
    --eth-wallet-private-key=$ETH_WALLET_PRIVATE_KEY \
    --eth-rpc-url=http://localhost:8545 \
    --execution-block=$BLOCK_NUMBER
```

### Testnet Deployment
```bash
# Deploy to Sepolia
forge script --rpc-url https://ethereum-sepolia-rpc.publicnode.com --broadcast Deploy
```

## Architecture

### Project Structure
- `apps/` - Rust host application that generates proofs and interacts with contracts
- `methods/` - zkVM guest code that runs inside RISC Zero VM
- `contracts/` - Solidity smart contracts
- `lib/` - External dependencies (git submodules)

### Key Components

1. **Smart Contracts** (`contracts/src/`)
   - Verify RISC Zero proofs on-chain
   - Store and validate economic data proofs
   - Use Steel commitments for state validation

2. **Publisher Applications** (`apps/src/bin/`)
   - Generate zkVM proofs for economic data off-chain
   - Submit proofs to smart contracts
   - Handle Ethereum transactions via Alloy
   - Example: `compound_apr_publisher` generates proofs for Compound APR data

3. **Guest Programs** (`methods/guest/src/bin/`)
   - Execute inside zkVM to compute economic metrics
   - Generate cryptographic commitments
   - Use Steel library for Ethereum state access

### Build Process Flow
1. `methods/build.rs` generates `ImageID.sol` and `Elf.sol` from zkVM guest code
2. These generated files are imported by the smart contracts
3. Publisher applications use the compiled guest ELF to generate proofs for economic data

## Environment Variables

### Required for Deployment/Testing
- `ETH_WALLET_ADDRESS` - Ethereum wallet address
- `ETH_WALLET_PRIVATE_KEY` - Private key for transactions
- `ETH_RPC_URL` - Ethereum RPC endpoint

### Optional
- `BONSAI_API_KEY` - For remote proving service
- `BONSAI_API_URL` - Bonsai service URL
- `RISC0_USE_DOCKER=1` - Use Docker for reproducible builds
- `BEACON_API_URL` - For beacon chain integration
- `HISTORY_BLOCKS` - Number of blocks for history proofs
- `COMMITMENT_BLOCK` - Block number for state commitments
- `EXECUTION_BLOCK` - Block number for economic data calculations

## Important Notes

- Generated files (`ImageID.sol`, `Elf.sol`) are gitignored and rebuilt automatically
- EVM version is set to 'cancun' in foundry.toml
- Default RPC endpoints configured: mainnet uses publicnode.com
- Rust toolchain: stable with clippy, rustfmt, rust-src components
- Uses Apache 2.0 license throughout