#![cfg(test)]

use super::*;
use soroban_sdk::testutils::{Address as _, Env as _};
use soroban_sdk::{Address, Env};

fn create_test_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}

fn setup_contract(env: &Env) -> Address {
    let contract_id = env.register_contract(None, FeeCollectorContract);
    let admin = Address::generate(env);

    let client = FeeCollectorContractClient::new(env, &contract_id);
    client.initialize(&admin);

    contract_id
}

#[test]
fn test_normal_trade() {
    let env = create_test_env();
    let contract_id = setup_contract(&env);
    let client = FeeCollectorContractClient::new(&env, &contract_id);

    let trade_amount = 10_000_000_000; // 1000 XLM
    let calculated_fee = 10_000_000;   // 1 XLM

    let result = client.collect_fee(&trade_amount, &calculated_fee);
    assert_eq!(result, 10_000_000); // Should return the calculated fee as is
}

#[test]
fn test_large_trade_cap() {
    let env = create_test_env();
    let contract_id = setup_contract(&env);
    let client = FeeCollectorContractClient::new(&env, &contract_id);

    let trade_amount = 1_000_000_000_000; // 100,000 XLM
    let calculated_fee = 2_000_000_000;   // 200 XLM (above max)

    let result = client.collect_fee(&trade_amount, &calculated_fee);
    assert_eq!(result, 1_000_000_000); // Should be capped at max_fee_per_trade (100 XLM)
}

#[test]
fn test_small_trade_floor() {
    let env = create_test_env();
    let contract_id = setup_contract(&env);
    let client = FeeCollectorContractClient::new(&env, &contract_id);

    let trade_amount = 1_000_000_000; // 100 XLM
    let calculated_fee = 10_000;       // 0.001 XLM (below min)

    let result = client.collect_fee(&trade_amount, &calculated_fee);
    assert_eq!(result, 100_000); // Should be floored at min_fee_per_trade (0.01 XLM)
}

#[test]
fn test_tiny_trade_reject() {
    let env = create_test_env();
    let contract_id = setup_contract(&env);
    let client = FeeCollectorContractClient::new(&env, &contract_id);

    let trade_amount = 50_000; // 0.005 XLM (below min_fee_per_trade)
    let calculated_fee = 5_000;

    let result = client.try_collect_fee(&trade_amount, &calculated_fee);
    assert_eq!(result, Err(Ok(FeeCollectorError::TradeTooSmall)));
}

#[test]
fn test_set_fee_config() {
    let env = create_test_env();
    let contract_id = setup_contract(&env);
    let client = FeeCollectorContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_config = FeeConfig {
        max_fee_per_trade: 2_000_000_000, // 200 XLM
        min_fee_per_trade: 200_000,       // 0.02 XLM
    };

    client.set_fee_config(&admin, &new_config);

    let retrieved_config = client.get_fee_config();
    assert_eq!(retrieved_config, new_config);
}