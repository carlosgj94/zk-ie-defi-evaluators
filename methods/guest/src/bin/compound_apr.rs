#![allow(unused_doc_comments)]
#![no_main]

use alloy_primitives::{address, aliases::U24, Address, Bytes, U160, U256};
use alloy_sol_types::{sol, SolValue};
use risc0_steel::{
    ethereum::{EthEvmInput, ETH_MAINNET_CHAIN_SPEC},
    Commitment, Contract,
};
use risc0_zkvm::guest::env;

risc0_zkvm::guest::entry!(main);

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

fn main() {
    let input: EthEvmInput = env::read();

    // Converts the input into a `EvmEnv` for execution. The `with_chain_spec` method is used
    // to specify the chain configuration. It checks that the state matches the state root in the
    // header provided in the input.
    let env = input.into_env().with_chain_spec(&ETH_MAINNET_CHAIN_SPEC);

    let cusdc_contract = Contract::new(CUSDC_COMMET, &env);
    let utilization = cusdc_contract
        .call_builder(&CometMainInterface::getUtilizationCall {})
        .call()
        ._0;
    let supply_rate = cusdc_contract
        .call_builder(&CometMainInterface::getSupplyRateCall { utilization })
        .call()
        ._0;
    let borrow_rate = cusdc_contract
        .call_builder(&CometMainInterface::getBorrowRateCall { utilization })
        .call()
        ._0;

    let supply_apr = supply_rate * SECONDS_PER_YEAR;
    let borrow_apr = borrow_rate * SECONDS_PER_YEAR;

    // Calculating the APR on COMP rewards
    let total_supply = cusdc_contract
        .call_builder(&CometMainInterface::totalSupplyCall {})
        .call()
        ._0;
    let total_borrow = cusdc_contract
        .call_builder(&CometMainInterface::totalBorrowCall {})
        .call()
        ._0;
    let base_tracking_supply_speed = cusdc_contract
        .call_builder(&CometMainInterface::baseTrackingSupplySpeedCall {})
        .call()
        ._0;

    let base_tracking_borrow_speed = cusdc_contract
        .call_builder(&CometMainInterface::baseTrackingBorrowSpeedCall {})
        .call()
        ._0;

    let quoter_contract_v2 = Contract::new(QUOTER_V2, &env);
    let mut path = Vec::new();
    path.extend_from_slice(COMP_ADDRESS.as_slice());
    path.extend_from_slice(&3000u32.to_be_bytes()[1..]); // 3000 fee tier (0.3%)
    path.extend_from_slice(WETH_ADDRESS.as_slice());
    path.extend_from_slice(&500u32.to_be_bytes()[1..]); // 500 fee tier (0.05%)
                                                        //    path.extend_from_slice(USDC_ADDRESS.as_slice());
    let path_bytes = Bytes::from(path);

    let comp_price = quoter_contract_v2
        .call_builder(&QuoterV2::quoteExactInputCall {
            path: path_bytes,
            amountIn: U256::from(1e18),
        })
        .call()
        .amountOut;

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

    // Commit the block hash and number used when deriving `view_call_env` to the journal.
    let journal = Journal {
        commitment: env.into_commitment(),
        annualBaseSupplyRate: supply_apr,
        annualCompRewardsSupplyRate: supply_rewards_apr,
        annualBaseBorrowRate: borrow_apr,
        annualCompRewardsBorrowRate: borrow_rewards_apr,
    };

    env::commit_slice(&journal.abi_encode());
}
