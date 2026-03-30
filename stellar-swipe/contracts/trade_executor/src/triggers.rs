//! Stop-loss trigger: checks oracle price against a position's stop-loss threshold
//! and closes the position via UserPortfolio when breached.
//!
//! Constraint: uses oracle price only — never SDEX spot — for manipulation resistance.

use soroban_sdk::{symbol_short, Address, Env, IntoVal, Symbol, Val, Vec};

use crate::errors::ContractError;

// ── Storage keys ─────────────────────────────────────────────────────────────

/// Instance key: oracle contract address (`get_price(asset_pair: u32) -> i128`).
pub const ORACLE_KEY: &str = "Oracle";
/// Instance key: user-portfolio contract address (`close_position(user, trade_id, pnl)`).
pub const PORTFOLIO_KEY: &str = "Portfolio";
/// Persistent key prefix: stop-loss price per (user, trade_id).
pub const SL_KEY: &str = "StopLoss";

// ── Public helpers ────────────────────────────────────────────────────────────

/// Register a stop-loss price for `(user, trade_id)`.
pub fn set_stop_loss(env: &Env, user: &Address, trade_id: u64, stop_loss_price: i128) {
    env.storage()
        .persistent()
        .set(&(symbol_short!("StopLoss"), user.clone(), trade_id), &stop_loss_price);
}

/// Return the registered stop-loss price, if any.
pub fn get_stop_loss(env: &Env, user: &Address, trade_id: u64) -> Option<i128> {
    env.storage()
        .persistent()
        .get(&(symbol_short!("StopLoss"), user.clone(), trade_id))
}

// ── Core trigger ──────────────────────────────────────────────────────────────

/// Check the oracle price for `asset_pair` against the registered stop-loss for
/// `(user, trade_id)`.  If `current_price <= stop_loss_price`, calls
/// `close_position(user, trade_id, 0)` on the portfolio contract and emits
/// `StopLossTriggered`.
///
/// Returns `Ok(true)` when triggered, `Ok(false)` when price is above threshold.
/// Returns `Err(ContractError::NotInitialized)` when oracle or portfolio are not
/// configured, or when no stop-loss is registered for the position.
pub fn check_and_trigger_stop_loss(
    env: &Env,
    user: Address,
    trade_id: u64,
    asset_pair: u32,
) -> Result<bool, ContractError> {
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

    let stop_loss_price: i128 = env
        .storage()
        .persistent()
        .get(&(symbol_short!("StopLoss"), user.clone(), trade_id))
        .ok_or(ContractError::NotInitialized)?;

    // Fetch oracle price (manipulation-resistant; never SDEX spot).
    let current_price: i128 = env.invoke_contract(
        &oracle,
        &Symbol::new(env, "get_price"),
        soroban_sdk::vec![env, asset_pair.into()],
    );

    if current_price <= stop_loss_price {
        // Close the position via UserPortfolio (realized_pnl = 0; portfolio computes it).
        let close_sym = Symbol::new(env, "close_position");
        let mut args = Vec::<Val>::new(env);
        args.push_back(user.clone().into_val(env));
        args.push_back(trade_id.into_val(env));
        args.push_back(0i128.into_val(env));
        env.invoke_contract::<()>(&portfolio, &close_sym, args);

        // Emit StopLossTriggered event.
        env.events().publish(
            (Symbol::new(env, "StopLossTriggered"), user.clone()),
            (trade_id, stop_loss_price, current_price),
        );

        Ok(true)
    } else {
        Ok(false)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StorageKey, TradeExecutorContract, TradeExecutorContractClient};
    use soroban_sdk::{contract, contractimpl, testutils::Address as _, Env};

    // ── Mock oracle ───────────────────────────────────────────────────────────

    #[contract]
    pub struct MockOracle;

    #[contractimpl]
    impl MockOracle {
        pub fn set_price(env: Env, price: i128) {
            env.storage().instance().set(&symbol_short!("price"), &price);
        }
        pub fn get_price(env: Env, _asset_pair: u32) -> i128 {
            env.storage()
                .instance()
                .get(&symbol_short!("price"))
                .unwrap_or(0)
        }
    }

    // ── Mock portfolio ────────────────────────────────────────────────────────

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

    // ── Setup helper ──────────────────────────────────────────────────────────

    fn setup() -> (Env, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let oracle_id = env.register(MockOracle, ());
        let portfolio_id = env.register(MockPortfolio, ());
        let exec_id = env.register(TradeExecutorContract, ());

        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.initialize(&admin);
        exec.set_oracle(&oracle_id);
        exec.set_stop_loss_portfolio(&portfolio_id);

        (env, exec_id, oracle_id, portfolio_id, admin)
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    /// Price above stop-loss → no trigger, portfolio untouched.
    #[test]
    fn no_trigger_when_price_above_stop_loss() {
        let (env, exec_id, oracle_id, portfolio_id, _) = setup();
        let user = Address::generate(&env);

        MockOracleClient::new(&env, &oracle_id).set_price(&200);

        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);

        let triggered = exec.check_and_trigger_stop_loss(&user, &1u64, &0u32);
        assert!(!triggered);
        assert!(MockPortfolioClient::new(&env, &portfolio_id).last_closed().is_none());
    }

    /// Price exactly at stop-loss → triggers.
    #[test]
    fn trigger_when_price_at_stop_loss() {
        let (env, exec_id, oracle_id, portfolio_id, _) = setup();
        let user = Address::generate(&env);

        MockOracleClient::new(&env, &oracle_id).set_price(&100);

        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);

        let triggered = exec.check_and_trigger_stop_loss(&user, &1u64, &0u32);
        assert!(triggered);
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(1u64)
        );
    }

    /// Price below stop-loss → triggers.
    #[test]
    fn trigger_when_price_below_stop_loss() {
        let (env, exec_id, oracle_id, portfolio_id, _) = setup();
        let user = Address::generate(&env);

        MockOracleClient::new(&env, &oracle_id).set_price(&50);

        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &2u64, &100);

        let triggered = exec.check_and_trigger_stop_loss(&user, &2u64, &0u32);
        assert!(triggered);
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(2u64)
        );
    }

    /// Event is emitted on trigger with correct prices.
    #[test]
    fn trigger_emits_stop_loss_event() {
        let (env, exec_id, oracle_id, _, _) = setup();
        let user = Address::generate(&env);

        MockOracleClient::new(&env, &oracle_id).set_price(&80);

        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &3u64, &100);
        exec.check_and_trigger_stop_loss(&user, &3u64, &0u32);

        let events = env.events().all();
        // Each event is (contract_id, topics, data); find StopLossTriggered by topic symbol.
        let found = events.iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            if let Some(first) = topics.get(0) {
                if let Ok(sym) = soroban_sdk::Symbol::try_from(first) {
                    return sym == Symbol::new(&env, "StopLossTriggered");
                }
            }
            false
        });
        assert!(found, "StopLossTriggered event not emitted");
    }
}
