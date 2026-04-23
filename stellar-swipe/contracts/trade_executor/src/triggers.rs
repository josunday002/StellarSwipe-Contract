//! Stop-loss and take-profit triggers: check oracle price against position
//! thresholds and close the position via UserPortfolio when breached.
//!
//! Constraint: uses oracle price only, never SDEX spot.
//! Priority: if both stop-loss and take-profit would trigger, stop-loss wins.

use soroban_sdk::{symbol_short, Address, Env, IntoVal, Symbol, Val, Vec};
use stellar_swipe_common::{
    oracle_price_to_i128, validate_freshness, IOracleClient, OnChainOracleClient,
};

use crate::errors::ContractError;

/// Instance key: oracle contract address (`get_price(asset_pair: u32) -> OraclePrice`).
pub const ORACLE_KEY: &str = "Oracle";
/// Instance key: user-portfolio contract address (`close_position(user, trade_id, pnl)`).
pub const PORTFOLIO_KEY: &str = "Portfolio";

const STOP_LOSS_KEY: soroban_sdk::Symbol = symbol_short!("StopLoss");
const TAKE_PROFIT_KEY: soroban_sdk::Symbol = symbol_short!("TakePrft");

/// Register a stop-loss price for `(user, trade_id)`.
pub fn set_stop_loss(env: &Env, user: &Address, trade_id: u64, stop_loss_price: i128) {
    env.storage()
        .persistent()
        .set(&(STOP_LOSS_KEY, user.clone(), trade_id), &stop_loss_price);
}

/// Return the registered stop-loss price, if any.
pub fn get_stop_loss(env: &Env, user: &Address, trade_id: u64) -> Option<i128> {
    env.storage()
        .persistent()
        .get(&(STOP_LOSS_KEY, user.clone(), trade_id))
}

/// Register a take-profit price for `(user, trade_id)`.
pub fn set_take_profit(env: &Env, user: &Address, trade_id: u64, take_profit_price: i128) {
    env.storage().persistent().set(
        &(TAKE_PROFIT_KEY, user.clone(), trade_id),
        &take_profit_price,
    );
}

/// Return the registered take-profit price, if any.
pub fn get_take_profit(env: &Env, user: &Address, trade_id: u64) -> Option<i128> {
    env.storage()
        .persistent()
        .get(&(TAKE_PROFIT_KEY, user.clone(), trade_id))
}

fn fetch_oracle_and_portfolio(env: &Env) -> Result<(Address, Address), ContractError> {
    let oracle: Address = env
        .storage()
        .instance()
        .get(&Symbol::new(env, ORACLE_KEY))
        .ok_or(ContractError::NotInitialized)?;
    let portfolio: Address = env
        .storage()
        .instance()
        .get(&Symbol::new(env, PORTFOLIO_KEY))
        .ok_or(ContractError::NotInitialized)?;
    Ok((oracle, portfolio))
}

fn close_position(env: &Env, portfolio: &Address, user: &Address, trade_id: u64) {
    let close_sym = Symbol::new(env, "close_position");
    let mut args = Vec::<Val>::new(env);
    args.push_back(user.clone().into_val(env));
    args.push_back(trade_id.into_val(env));
    args.push_back(0i128.into_val(env));
    env.invoke_contract::<()>(portfolio, &close_sym, args);
}

fn fetch_current_price(
    env: &Env,
    oracle: &Address,
    asset_pair: u32,
) -> Result<i128, ContractError> {
    let client = OnChainOracleClient {
        address: oracle.clone(),
    };
    let price = client
        .get_price(env, asset_pair)
        .map_err(|_| ContractError::NotInitialized)?;
    validate_freshness(env, &price).map_err(|_| ContractError::NotInitialized)?;
    Ok(oracle_price_to_i128(&price))
}

/// Check oracle price against stop-loss for `(user, trade_id)`.
/// If `current_price <= stop_loss_price`, closes the position and emits
/// `StopLossTriggered`.
pub fn check_and_trigger_stop_loss(
    env: &Env,
    user: Address,
    trade_id: u64,
    asset_pair: u32,
) -> Result<bool, ContractError> {
    let (oracle, portfolio) = fetch_oracle_and_portfolio(env)?;
    let stop_loss_price =
        get_stop_loss(env, &user, trade_id).ok_or(ContractError::NotInitialized)?;
    let current_price = fetch_current_price(env, &oracle, asset_pair)?;

    if current_price <= stop_loss_price {
        close_position(env, &portfolio, &user, trade_id);
        env.events().publish(
            (Symbol::new(env, "StopLossTriggered"), user.clone()),
            (trade_id, stop_loss_price, current_price),
        );
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Check oracle price against take-profit for `(user, trade_id)`.
/// If `current_price >= take_profit_price`, closes the position and emits
/// `TakeProfitTriggered`. Stop-loss takes priority when both would trigger.
pub fn check_and_trigger_take_profit(
    env: &Env,
    user: Address,
    trade_id: u64,
    asset_pair: u32,
) -> Result<bool, ContractError> {
    let (oracle, portfolio) = fetch_oracle_and_portfolio(env)?;
    let take_profit_price =
        get_take_profit(env, &user, trade_id).ok_or(ContractError::NotInitialized)?;
    let current_price = fetch_current_price(env, &oracle, asset_pair)?;

    if let Some(stop_loss_price) = get_stop_loss(env, &user, trade_id) {
        if current_price <= stop_loss_price {
            return Ok(false);
        }
    }

    if current_price >= take_profit_price {
        close_position(env, &portfolio, &user, trade_id);
        env.events().publish(
            (Symbol::new(env, "TakeProfitTriggered"), user.clone()),
            (trade_id, take_profit_price, current_price),
        );
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TradeExecutorContract, TradeExecutorContractClient};
    use soroban_sdk::{
        contract, contractimpl,
        testutils::{Address as _, Events, Ledger},
        Env, TryFromVal,
    };
    use stellar_swipe_common::OraclePrice;

    #[contract]
    pub struct MockOracle;

    #[contractimpl]
    impl MockOracle {
        pub fn set_price(env: Env, asset_pair: u32, price: OraclePrice) {
            env.storage()
                .instance()
                .set(&(symbol_short!("price"), asset_pair), &price);
        }

        pub fn get_price(env: Env, asset_pair: u32) -> OraclePrice {
            env.storage()
                .instance()
                .get(&(symbol_short!("price"), asset_pair))
                .unwrap()
        }
    }

    #[contract]
    pub struct MockPortfolio;

    #[contractimpl]
    impl MockPortfolio {
        pub fn close_position(env: Env, _user: Address, trade_id: u64, _pnl: i128) {
            env.storage()
                .instance()
                .set(&symbol_short!("closed"), &trade_id);
        }

        pub fn last_closed(env: Env) -> Option<u64> {
            env.storage().instance().get(&symbol_short!("closed"))
        }
    }

    fn set_oracle_price(env: &Env, oracle_id: &Address, asset_pair: u32, price: i128) {
        MockOracleClient::new(env, oracle_id).set_price(
            &asset_pair,
            &OraclePrice {
                price: price * 100,
                decimals: 2,
                timestamp: env.ledger().timestamp(),
                source: Symbol::new(env, "mock"),
            },
        );
    }

    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|ledger| ledger.timestamp = 1_000);

        let admin = Address::generate(&env);
        let oracle_id = env.register(MockOracle, ());
        let portfolio_id = env.register(MockPortfolio, ());
        let exec_id = env.register(TradeExecutorContract, ());

        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.initialize(&admin);
        exec.set_oracle(&oracle_id);
        exec.set_stop_loss_portfolio(&portfolio_id);

        (env, exec_id, oracle_id, portfolio_id)
    }

    fn event_was_emitted(env: &Env, expected: &str) -> bool {
        env.events().all().iter().any(|event| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = event.1.clone();
            topics
                .get(0)
                .and_then(|val| soroban_sdk::Symbol::try_from_val(env, &val).ok())
                .map(|symbol| symbol == Symbol::new(env, expected))
                .unwrap_or(false)
        })
    }

    #[test]
    fn no_trigger_when_price_above_stop_loss() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);

        set_oracle_price(&env, &oracle_id, 7, 200);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);

        assert!(!exec.check_and_trigger_stop_loss(&user, &1u64, &7u32));
        assert!(MockPortfolioClient::new(&env, &portfolio_id)
            .last_closed()
            .is_none());
    }

    #[test]
    fn trigger_when_price_at_stop_loss() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);

        set_oracle_price(&env, &oracle_id, 7, 100);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);

        assert!(exec.check_and_trigger_stop_loss(&user, &1u64, &7u32));
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(1u64)
        );
    }

    #[test]
    fn trigger_when_price_below_stop_loss() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);

        set_oracle_price(&env, &oracle_id, 7, 50);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &2u64, &100);

        assert!(exec.check_and_trigger_stop_loss(&user, &2u64, &7u32));
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(2u64)
        );
    }

    #[test]
    fn stop_loss_trigger_emits_event() {
        let (env, exec_id, oracle_id, _) = setup();
        let user = Address::generate(&env);

        set_oracle_price(&env, &oracle_id, 7, 80);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &3u64, &100);

        exec.check_and_trigger_stop_loss(&user, &3u64, &7u32);
        assert!(event_was_emitted(&env, "StopLossTriggered"));
    }

    #[test]
    fn no_trigger_when_price_below_take_profit() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);

        set_oracle_price(&env, &oracle_id, 7, 150);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &1u64, &200);

        assert!(!exec.check_and_trigger_take_profit(&user, &1u64, &7u32));
        assert!(MockPortfolioClient::new(&env, &portfolio_id)
            .last_closed()
            .is_none());
    }

    #[test]
    fn trigger_when_price_at_take_profit() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);

        set_oracle_price(&env, &oracle_id, 7, 200);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &1u64, &200);

        assert!(exec.check_and_trigger_take_profit(&user, &1u64, &7u32));
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(1u64)
        );
    }

    #[test]
    fn trigger_when_price_above_take_profit() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);

        set_oracle_price(&env, &oracle_id, 7, 250);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &2u64, &200);

        assert!(exec.check_and_trigger_take_profit(&user, &2u64, &7u32));
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(2u64)
        );
    }

    #[test]
    fn take_profit_trigger_emits_event() {
        let (env, exec_id, oracle_id, _) = setup();
        let user = Address::generate(&env);

        set_oracle_price(&env, &oracle_id, 7, 300);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &3u64, &200);

        exec.check_and_trigger_take_profit(&user, &3u64, &7u32);
        assert!(event_was_emitted(&env, "TakeProfitTriggered"));
    }

    #[test]
    fn stop_loss_priority_over_take_profit_on_simultaneous_trigger() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);

        set_oracle_price(&env, &oracle_id, 7, 50);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);
        exec.set_take_profit_price(&user, &1u64, &50);

        assert!(!exec.check_and_trigger_take_profit(&user, &1u64, &7u32));
        assert!(MockPortfolioClient::new(&env, &portfolio_id)
            .last_closed()
            .is_none());

        assert!(exec.check_and_trigger_stop_loss(&user, &1u64, &7u32));
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(1u64)
        );
    }
}
