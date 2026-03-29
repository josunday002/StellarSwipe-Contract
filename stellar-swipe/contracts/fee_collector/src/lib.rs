#![no_std]

use soroban_sdk::{contract, contractimpl, contracterror, contracttype, Address, Env, Symbol};

use stellar_swipe_common::assets::Asset;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FeeCollectorError {
    TradeTooSmall = 1,
    ArithmeticOverflow = 2,
    InvalidAmount = 3,
    Unauthorized = 4,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeeConfig {
    pub max_fee_per_trade: i128, // 100 XLM equivalent
    pub min_fee_per_trade: i128, // 0.01 XLM equivalent
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataKey {
    Admin,
    FeeConfig,
}

#[contract]
pub struct FeeCollectorContract;

#[contractimpl]
impl FeeCollectorContract {
    /// Initialize the contract with admin and default fee config
    pub fn initialize(env: Env, admin: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);

        // Default config: 100 XLM = 100 * 10^7 stroops = 1_000_000_000
        // 0.01 XLM = 0.01 * 10^7 = 100_000
        let default_config = FeeConfig {
            max_fee_per_trade: 1_000_000_000, // 100 XLM
            min_fee_per_trade: 100_000,       // 0.01 XLM
        };
        env.storage().instance().set(&DataKey::FeeConfig, &default_config);
    }

    /// Set fee config (admin only)
    pub fn set_fee_config(env: Env, caller: Address, config: FeeConfig) -> Result<(), FeeCollectorError> {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if caller != admin {
            return Err(FeeCollectorError::Unauthorized);
        }

        if config.min_fee_per_trade <= 0 || config.max_fee_per_trade <= config.min_fee_per_trade {
            return Err(FeeCollectorError::InvalidAmount);
        }

        env.storage().instance().set(&DataKey::FeeConfig, &config);
        Ok(())
    }

    /// Get current fee config
    pub fn get_fee_config(env: Env) -> FeeConfig {
        env.storage().instance().get(&DataKey::FeeConfig).unwrap()
    }

    /// Collect fee with cap and floor applied
    /// Returns the clamped fee amount
    pub fn collect_fee(env: Env, trade_amount: i128, calculated_fee: i128) -> Result<i128, FeeCollectorError> {
        if trade_amount <= 0 || calculated_fee < 0 {
            return Err(FeeCollectorError::InvalidAmount);
        }

        let config = Self::get_fee_config(env);

        // Check if trade amount is too small to cover minimum fee
        if trade_amount < config.min_fee_per_trade {
            return Err(FeeCollectorError::TradeTooSmall);
        }

        // Clamp the fee between min and max
        let clamped_fee = if calculated_fee < config.min_fee_per_trade {
            config.min_fee_per_trade
        } else if calculated_fee > config.max_fee_per_trade {
            config.max_fee_per_trade
        } else {
            calculated_fee
        };

        // Ensure the clamped fee doesn't exceed the trade amount
        if clamped_fee > trade_amount {
            return Err(FeeCollectorError::TradeTooSmall);
        }

        Ok(clamped_fee)
    }
}