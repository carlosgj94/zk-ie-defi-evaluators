use alloy_primitives::{Address, U256};
use anyhow::{ensure, Context, Result};
use clap::Parser;
use erc20_counter_methods::{BALANCE_OF_ELF, BALANCE_OF_ID};
use risc0_ethereum_contracts::encode_seal;
use risc0_steel::{
    alloy::{
        network::EthereumWallet,
        providers::ProviderBuilder,
        signers::local::PrivateKeySigner,
        sol,
        sol_types::{SolCall, SolValue},
    },
    ethereum::ETH_MAINNET_CHAIN_SPEC,
};
use risc0_steel::{ethereum::EthEvmEnv, host::BlockNumberOrTag, Commitment, Contract};
use risc0_zkvm::{default_prover, Digest, ExecutorEnv, ProverOpts, VerifierContext};
use tokio::task;
use tracing_subscriber::EnvFilter;
use url::Url;

sol! {
    /// Interface to be called by the guest.
    interface IERC20 {
        function balanceOf(address account) external view returns (uint);
        function totalSupply() external view returns (uint256);
    }

    /// Data committed to by the guest.
    struct Journal {
        Commitment commitment;
        address tokenContract;
        uint256 circulatingSupply;
        uint256 pastCirculatingSupply;
        uint256 inflationBasisPoints;
    }
}

/// Simple program to create a proof to increment the Counter contract.
#[derive(Parser)]
struct Args {
    /// Ethereum private key
    #[arg(long, env = "ETH_WALLET_PRIVATE_KEY")]
    eth_wallet_private_key: PrivateKeySigner,

    /// Ethereum RPC endpoint URL
    #[arg(long, env = "ETH_RPC_URL")]
    eth_rpc_url: Url,

    /// Ethereum block to use as the state for the contract call
    #[arg(long, env = "EXECUTION_BLOCK", default_value_t = BlockNumberOrTag::Parent)]
    execution_block: BlockNumberOrTag,

    /// Ethereum block to use as the state for the contract call
    #[arg(long, env = "PAST_EXECUTION_BLOCK", default_value_t = BlockNumberOrTag::Parent)]
    past_execution_block: BlockNumberOrTag,

    /// Ethereum block to use for the beacon block commitment.
    #[cfg(feature = "history")]
    #[arg(long, env = "COMMITMENT_BLOCK")]
    commitment_block: BlockNumberOrTag,

    /// Address of the ERC20 token contract
    #[arg(long)]
    token_contract: Address,

    /// Address to query the token balance of
    #[arg(long)]
    account: Address,

    /// Address to query the token balance of
    #[arg(long)]
    account_2: Address,
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

    let past_builder = EthEvmEnv::builder()
        .provider(provider.clone())
        .block_number_or_tag(args.past_execution_block);

    #[cfg(any(feature = "beacon", feature = "history"))]
    let builder = builder.beacon_api(args.beacon_api_url);
    #[cfg(feature = "history")]
    let builder = builder.commitment_block_number_or_tag(args.commitment_block);

    let mut env = builder.build().await?;
    let mut past_env = past_builder.build().await?;
    //  The `with_chain_spec` method is used to specify the chain configuration.

    env = env.with_chain_spec(&ETH_MAINNET_CHAIN_SPEC);
    past_env = past_env.with_chain_spec(&ETH_MAINNET_CHAIN_SPEC);

    /*
        // Prepare the function call
        let call = IERC20::balanceOfCall {
            account: args.account,
        };

        // Preflight the call to prepare the input that is required to execute the function in
        // the guest without RPC access. It also returns the result of the call.
        let mut contract = Contract::preflight(args.token_contract, &mut env);
        let returns = contract.call_builder(&call).call().await?._0;
    */
    let supply_call = IERC20::totalSupplyCall {};
    let balance_of_call = IERC20::balanceOfCall {
        account: args.account,
    };
    let balance_of_call_2 = IERC20::balanceOfCall {
        account: args.account_2,
    };

    ///// Present Supply
    let mut token_contract = Contract::preflight(args.token_contract, &mut env);
    // let returns = Contract::new(contract, &env).call_builder(&call).call();
    let total_supply = token_contract.call_builder(&supply_call).call().await?._0;
    let balance = token_contract
        .call_builder(&balance_of_call)
        .call()
        .await?
        ._0;
    let balance_2 = token_contract
        .call_builder(&balance_of_call_2)
        .call()
        .await?
        ._0;

    let circulating_supply = total_supply - balance - balance_2;

    ///// Past Supply
    let mut token_contract = Contract::preflight(args.token_contract, &mut past_env);
    // let returns = Contract::new(contract, &env).call_builder(&call).call();
    let past_total_supply = token_contract.call_builder(&supply_call).call().await?._0;
    let past_balance = token_contract
        .call_builder(&balance_of_call)
        .call()
        .await?
        ._0;
    let past_balance_2 = token_contract
        .call_builder(&balance_of_call_2)
        .call()
        .await?
        ._0;

    let past_circulating_supply = past_total_supply - past_balance - past_balance_2;

    let inflation_basis_points = ((circulating_supply - past_circulating_supply)
        * U256::from(10000))
        / past_circulating_supply;

    // Finally, construct the input from the environment.
    // There are two options: Use EIP-4788 for verification by providing a Beacon API endpoint,
    // or use the regular `blockhash' opcode.
    let evm_input = env.into_input().await?;
    let past_evm_input = past_env.into_input().await?;

    // Create the steel proof.
    let prove_info = task::spawn_blocking(move || {
        let env = ExecutorEnv::builder()
            .write(&evm_input)?
            .write(&past_evm_input)?
            .write(&args.token_contract)?
            .write(&args.account)?
            .write(&args.account_2)?
            .build()
            .unwrap();

        default_prover().prove_with_ctx(
            env,
            &VerifierContext::default(),
            BALANCE_OF_ELF,
            &ProverOpts::groth16(),
        )
    })
    .await?
    .context("failed to create proof")?;
    let receipt = prove_info.receipt;
    let journal = &receipt.journal.bytes;

    // Decode and log the commitment
    let journal = Journal::abi_decode(journal, true).context("invalid journal")?;
    log::info!("Curve token: {:?}", args.token_contract);
    log::info!("Total Supply: {:?}", total_supply);
    log::info!("Circulating Supply: {:?}", circulating_supply);
    log::info!("Past Circulating Supply: {:?}", past_circulating_supply);
    log::info!("Inflation Basis Points: {:?}", inflation_basis_points);
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
    */

    Ok(())
}
