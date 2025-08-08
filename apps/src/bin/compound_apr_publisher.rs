use alloy_primitives::{address, Address, Bytes, U160, U256};
use anyhow::{ensure, Context, Result};
use clap::Parser;
use erc20_counter_methods::COMPOUND_APR_ELF;
use risc0_ethereum_contracts::encode_seal;
use risc0_steel::alloy::{
    network::EthereumWallet,
    providers::ProviderBuilder,
    signers::local::PrivateKeySigner,
    sol,
    sol_types::{SolCall, SolValue},
};
use risc0_steel::{
    ethereum::{EthEvmEnv, ETH_MAINNET_CHAIN_SPEC},
    host::BlockNumberOrTag,
    Commitment, Contract,
};
use risc0_zkvm::{default_prover, Digest, ExecutorEnv, ProverOpts, VerifierContext};
use tokio::task;
use tracing_subscriber::EnvFilter;
use url::Url;

sol! {
    interface QuoterV2 {
        function quoteExactInput(bytes memory path, uint256 amountIn) public returns(
            uint256 amountOut,
            uint160[] memory sqrtPriceX96AfterList,
            uint32[] memory initializedTicksCrossedList,
            uint256 gasEstimate
    );
    }
}

sol! {
    /// Simplified interface of the Compound Finance Comet contract
    interface CometMainInterface {
        function getSupplyRate(uint256 utilization) virtual public view returns (uint64);
        function getBorrowRate(uint256 utilization) virtual public view returns (uint64);
        function getUtilization() public view returns (uint256);

        function totalSupply() public view returns(uint256);
        function totalBorrow() public view returns(uint256);

        function baseTrackingSupplySpeed() public view returns(uint256);
        function baseTrackingBorrowSpeed() public view returns(uint256);

    }
}

sol! {
    struct Journal {
        Commitment commitment;
        uint64 annualBaseSupplyRate;
        uint256 annualCompRewardsSupplyRate;
        uint64 annualBaseBorrowRate;
        uint256 annualCompRewardsBorrowRate;
    }
}

const SECONDS_PER_YEAR: u64 = 60 * 60 * 24 * 365;
const CUSDC_COMMET: Address = address!("c3d688B66703497DAA19211EEdff47f25384cdc3");
const QUOTER_V2: Address = address!("61fFE014bA17989E743c5F6cB21bF9697530B21e");
const COMP_ADDRESS: Address = address!("c00e94Cb662C3520282E6f5717214004A7f26888");
const WETH_ADDRESS: Address = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
const USDC_ADDRESS: Address = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");

/// Simple program to create a proof to increment the Counter contract.
#[derive(Parser)]
struct Args {
    /// Ethereum private key
    #[arg(long, env = "ETH_WALLET_PRIVATE_KEY")]
    eth_wallet_private_key: PrivateKeySigner,

    /// Ethereum RPC endpoint URL
    #[arg(long, env = "ETH_RPC_URL")]
    eth_rpc_url: Url,

    /// Beacon API endpoint URL
    ///
    /// Steel uses a beacon block commitment instead of the execution block.
    /// This allows proofs to be validated using the EIP-4788 beacon roots contract.
    #[cfg(any(feature = "beacon", feature = "history"))]
    #[arg(long, env = "BEACON_API_URL")]
    beacon_api_url: Url,

    /// Ethereum block to use as the state for the contract call
    #[arg(long, env = "EXECUTION_BLOCK", default_value_t = BlockNumberOrTag::Parent)]
    execution_block: BlockNumberOrTag,

    /// Ethereum block to use for the beacon block commitment.
    #[cfg(feature = "history")]
    #[arg(long, env = "COMMITMENT_BLOCK")]
    commitment_block: BlockNumberOrTag,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing. In order to view logs, run `RUST_LOG=info cargo run`
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    // Parse the command line arguments.
    let args = Args::try_parse()?;

    // Create an alloy provider for that private key and URL.
    let wallet = EthereumWallet::from(args.eth_wallet_private_key);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .on_http(args.eth_rpc_url);

    #[cfg(feature = "beacon")]
    log::info!("Beacon commitment to block {}", args.execution_block);
    #[cfg(feature = "history")]
    log::info!("History commitment to block {}", args.commitment_block);

    let builder = EthEvmEnv::builder()
        .provider(provider.clone())
        .block_number_or_tag(args.execution_block);
    #[cfg(any(feature = "beacon", feature = "history"))]
    let builder = builder.beacon_api(args.beacon_api_url);
    #[cfg(feature = "history")]
    let builder = builder.commitment_block_number_or_tag(args.commitment_block);

    let mut env = builder.build().await?;
    //  The `with_chain_spec` method is used to specify the chain configuration.
    env = env.with_chain_spec(&ETH_MAINNET_CHAIN_SPEC);

    // Preflight the call to prepare the input that is required to execute the function in
    // the guest without RPC access. It also returns the result of the call.
    let mut cusdc_contract = Contract::preflight(CUSDC_COMMET, &mut env);
    let utilization = cusdc_contract
        .call_builder(&CometMainInterface::getUtilizationCall {})
        .call()
        .await?
        ._0;
    let supply_rate = cusdc_contract
        .call_builder(&CometMainInterface::getSupplyRateCall { utilization })
        .call()
        .await?
        ._0;
    let borrow_rate = cusdc_contract
        .call_builder(&CometMainInterface::getBorrowRateCall { utilization })
        .call()
        .await?
        ._0;

    let supply_apr = supply_rate * SECONDS_PER_YEAR;
    let borrow_apr = borrow_rate * SECONDS_PER_YEAR;

    // Calculating the APR on COMP rewards
    let total_supply = cusdc_contract
        .call_builder(&CometMainInterface::totalSupplyCall {})
        .call()
        .await?
        ._0;
    let total_borrow = cusdc_contract
        .call_builder(&CometMainInterface::totalBorrowCall {})
        .call()
        .await?
        ._0;
    let base_tracking_supply_speed = cusdc_contract
        .call_builder(&CometMainInterface::baseTrackingSupplySpeedCall {})
        .call()
        .await?
        ._0;

    let base_tracking_borrow_speed = cusdc_contract
        .call_builder(&CometMainInterface::baseTrackingBorrowSpeedCall {})
        .call()
        .await?
        ._0;

    // Price calculation
    let mut quoter_contract_v2 = Contract::preflight(QUOTER_V2, &mut env);
    let mut path = Vec::new();
    path.extend_from_slice(COMP_ADDRESS.as_slice());
    path.extend_from_slice(&[0x00, 0x0B, 0xB8]); // 3000 in 3 bytes
    path.extend_from_slice(WETH_ADDRESS.as_slice());
    path.extend_from_slice(&[0x00, 0x01, 0xF4]); // 500 in 3 bytes
                                                 //    path.extend_from_slice(USDC_ADDRESS.as_slice());
    let path_bytes = Bytes::from(path);

    let comp_price = quoter_contract_v2
        .call_builder(&QuoterV2::quoteExactInputCall {
            path: path_bytes,
            amountIn: U256::from(1e18),
        })
        .call()
        .await?
        .amountOut;

    log::info!("COMP -  ETH - USDC: {:?}", comp_price);
    // End of price calculation

    let comp_scaling_factor = U256::from(1_000u128);

    let supply_rewards_apr = (base_tracking_supply_speed
        * U256::from(SECONDS_PER_YEAR)
        * comp_price
        * comp_scaling_factor)
        / total_supply;

    let borrow_rewards_apr = (base_tracking_borrow_speed
        * U256::from(SECONDS_PER_YEAR)
        * comp_price
        * comp_scaling_factor)
        / total_borrow;

    log::info!("Supply APR: {:?}", supply_apr); // This is in 1e18
    log::info!("Borrow APR: {:?}", borrow_apr); // This is in 1e18
    log::info!("Supply COMP Rewards APR: {:?}", supply_rewards_apr);
    log::info!("Borrow COMP Rewards APR: {:?}", borrow_rewards_apr);
    log::info!(
        "Total Supply APR: {:?}",
        U256::from(supply_apr) + supply_rewards_apr
    );
    log::info!(
        "Total Borrow APR: {:?}",
        U256::from(borrow_apr) - borrow_rewards_apr
    );

    // Finally, construct the input from the environment.
    // There are two options: Use EIP-4788 for verification by providing a Beacon API endpoint,
    // or use the regular `blockhash' opcode.
    let evm_input = env.into_input().await?;

    // Create the steel proof.
    let prove_info = task::spawn_blocking(move || {
        let env = ExecutorEnv::builder().write(&evm_input)?.build().unwrap();

        default_prover().prove_with_ctx(
            env,
            &VerifierContext::default(),
            COMPOUND_APR_ELF,
            &ProverOpts::groth16(),
        )
    })
    .await?
    .context("failed to create proof")?;
    let receipt = prove_info.receipt;
    let journal = &receipt.journal.bytes;

    // Decode and log the commitment
    let journal = Journal::abi_decode(journal, true).context("invalid journal")?;
    log::info!("Steel commitment: {:?}", journal.commitment);

    /*
    // ABI encode the seal.
    let seal = encode_seal(&receipt).context("invalid receipt")?;

        // Create an alloy instance of the Counter contract.
        let contract = ICounter::new(args.counter_address, &provider);

        // Call ICounter::imageID() to check that the contract has been deployed correctly.
        let contract_image_id = Digest::from(contract.imageID().call().await?._0.0);
        ensure!(contract_image_id == BALANCE_OF_ID.into());

        // Call the increment function of the contract and wait for confirmation.
        log::info!(
            "Sending Tx calling {} Function of {:#}...",
            ICounter::incrementCall::SIGNATURE,
            contract.address()
        );
        let call_builder = contract.increment(receipt.journal.bytes.into(), seal.into());
        log::debug!("Send {} {}", contract.address(), call_builder.calldata());
        let pending_tx = call_builder.send().await?;
        let tx_hash = *pending_tx.tx_hash();
        let receipt = pending_tx
            .get_receipt()
            .await
            .with_context(|| format!("transaction did not confirm: {}", tx_hash))?;
        ensure!(receipt.status(), "transaction failed: {}", tx_hash);
    **/
    Ok(())
}
