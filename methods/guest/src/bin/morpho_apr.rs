#![allow(unused_doc_comments)]
#![no_main]

use alloy_primitives::{address, hex, Address, FixedBytes, U128, U256};
use alloy_sol_types::{sol, SolValue};
use risc0_steel::{
    ethereum::{EthEvmInput, ETH_MAINNET_CHAIN_SPEC},
    Commitment, Contract,
};
use risc0_zkvm::guest::env;

risc0_zkvm::guest::entry!(main);

sol! {
    interface MorphoMarketInterface {
        function market(bytes32 id) public view returns(
            uint128 totalSupplyAssets,
            uint128 totalSupplyShares,
            uint128 totalBorrowAssets,
            uint128 totalBorrowShares,
            uint128 lastUpdate,
            uint128 fee
        );
        function idToMarketParameters(bytes32 id) public view returns(
            address loanToken,
            address collateralToken,
            address oracle,
            address irm,
            uint256 lltv
        );
    }
}

sol! {
    struct MarketParams {
        address loanToken;
        address collateralToken;
        address oracle;
        address irm;
        uint256 lltv;
    }
    struct Market {
        uint128 totalSupplyAssets;
        uint128 totalSupplyShares;
        uint128 totalBorrowAssets;
        uint128 totalBorrowShares;
        uint128 lastUpdate;
        uint128 fee;
    }
    interface IRMInterface {
        function borrowRateView(MarketParams marketParams, Market market) public view returns(uint256);
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
const MORPHO_MARKET: Address = address!("BBBBBbbBBb9cC5e90e3b3Af64bdAF62C37EEFFCb");
const STAKEHOUSE_USDC_MARKET_ID: FixedBytes<32> = FixedBytes(hex!(
    "b323495f7e4148be5643a4ea4a8221eef163e4bccfdedc2a6f4696baacbc86cc"
));

const WETH_ADDRESS: Address = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
const USDC_ADDRESS: Address = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");

fn main() {
    let input: EthEvmInput = env::read();

    // Converts the input into a `EvmEnv` for execution. The `with_chain_spec` method is used
    // to specify the chain configuration. It checks that the state matches the state root in the
    // header provided in the input.
    let env = input.into_env().with_chain_spec(&ETH_MAINNET_CHAIN_SPEC);

    let morpho_contract = Contract::new(MORPHO_MARKET, &env);
    let market = morpho_contract
        .call_builder(&MorphoMarketInterface::marketCall {
            id: STAKEHOUSE_USDC_MARKET_ID,
        })
        .call();
    let market_params = morpho_contract
        .call_builder(&MorphoMarketInterface::idToMarketParametersCall {
            id: STAKEHOUSE_USDC_MARKET_ID,
        })
        .call();

    let irm = Contract::new(market_params.irm, &env);
    let borrow_rate_per_second = irm
        .call_builder(&IRMInterface::borrowRateViewCall {
            marketParams: MarketParams {
                loanToken: market_params.loanToken,
                collateralToken: market_params.collateralToken,
                oracle: market_params.oracle,
                irm: market_params.irm,
                lltv: market_params.lltv,
            },
            market: Market {
                totalSupplyAssets: market.totalSupplyAssets,
                totalSupplyShares: market.totalSupplyShares,
                totalBorrowAssets: market.totalBorrowAssets,
                totalBorrowShares: market.totalBorrowShares,
                lastUpdate: market.lastUpdate,
                fee: market.fee,
            },
        })
        .call()
        ._0;

    let utilization = if market.totalSupplyAssets > 0 {
        (U256::from(market.totalBorrowAssets) * U256::from(10e18 as u128))
            / U256::from(market.totalSupplyAssets)
    } else {
        U256::ZERO
    };

    let supply_rate_per_second = borrow_rate_per_second * utilization;

    let supply_apr = supply_rate_per_second * U256::from(SECONDS_PER_YEAR);
    let borrow_apr = borrow_rate_per_second * U256::from(SECONDS_PER_YEAR);
}
