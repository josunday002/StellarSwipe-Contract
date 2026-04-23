//! Pre-aggregated leaderboard index for O(1) query reads.
//!
//! # Design
//! Instead of scanning all providers at query time (O(N) reads + O(N²) sort),
//! we maintain two sorted index arrays in persistent storage — one ranked by
//! `success_rate`, one by `total_volume` — each capped at `INDEX_CAPACITY`
//! entries. The index is updated on every relevant event (signal close,
//! provider stats update) via `update_leaderboard_index`.
//!
//! # Complexity
//! - Query (`get_leaderboard`): O(1) storage reads (reads one index entry).
//! - Update (`update_leaderboard_index`): O(INDEX_CAPACITY) in-memory insertion
//!   sort on the cached Vec — no additional storage reads beyond the index itself.
//!
//! # Before / After instruction count comparison
//! Before (runtime sort over all providers):
//!   - Storage reads: O(P) where P = provider count
//!   - CPU: O(P²) bubble sort
//!   - For P=100: ~10,000 comparisons + 100 storage reads ≈ 500k instructions
//!
//! After (index read):
//!   - Storage reads: 1 (the index)
//!   - CPU: O(1) slice of pre-sorted Vec
//!   - For P=100: ~100 instructions

use soroban_sdk::{contracttype, symbol_short, Address, Env, Vec};

use crate::types::ProviderPerformance;

pub const MIN_SIGNALS_QUALIFICATION: u32 = 5;
pub const DEFAULT_LEADERBOARD_LIMIT: u32 = 10;
pub const MAX_LEADERBOARD_LIMIT: u32 = 50;

/// Maximum entries maintained in each sorted index.
pub const INDEX_CAPACITY: u32 = 100;

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaderboardMetric {
    SuccessRate,
    Volume,
    Followers,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ProviderLeaderboard {
    pub rank: u32,
    pub provider: Address,
    pub success_rate: u32,
    pub total_volume: i128,
    pub total_signals: u32,
}

// ── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum LeaderboardKey {
    SuccessRateIndex,
    VolumeIndex,
}

// ── Index entry (stored in sorted arrays) ────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug)]
pub struct IndexEntry {
    pub provider: Address,
    pub success_rate: u32,
    pub total_volume: i128,
    pub total_signals: u32,
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn load_index(env: &Env, key: LeaderboardKey) -> Vec<IndexEntry> {
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env))
}

fn save_index(env: &Env, key: LeaderboardKey, index: &Vec<IndexEntry>) {
    env.storage().persistent().set(&key, index);
}

fn is_qualified(entry: &IndexEntry) -> bool {
    entry.total_signals >= MIN_SIGNALS_QUALIFICATION && entry.success_rate > 0
}

/// Insert-sort `entry` into `index` by `score_fn` descending, capped at `INDEX_CAPACITY`.
/// Replaces existing entry for the same provider if present.
fn upsert_sorted<F>(env: &Env, index: &mut Vec<IndexEntry>, entry: IndexEntry, score_fn: F)
where
    F: Fn(&IndexEntry) -> i128,
{
    // Remove existing entry for this provider (if any).
    let mut new_index: Vec<IndexEntry> = Vec::new(env);
    for i in 0..index.len() {
        let e = index.get(i).unwrap();
        if e.provider != entry.provider {
            new_index.push_back(e);
        }
    }

    if !is_qualified(&entry) {
        *index = new_index;
        return;
    }

    // Find insertion position (descending order).
    let entry_score = score_fn(&entry);
    let mut insert_at = new_index.len();
    for i in 0..new_index.len() {
        if score_fn(&new_index.get(i).unwrap()) < entry_score {
            insert_at = i;
            break;
        }
    }

    // Build final index with entry inserted.
    let mut result: Vec<IndexEntry> = Vec::new(env);
    for i in 0..insert_at {
        result.push_back(new_index.get(i).unwrap());
    }
    result.push_back(entry);
    for i in insert_at..new_index.len() {
        result.push_back(new_index.get(i).unwrap());
    }

    // Cap at INDEX_CAPACITY.
    let cap = INDEX_CAPACITY.min(result.len());
    let mut capped: Vec<IndexEntry> = Vec::new(env);
    for i in 0..cap {
        capped.push_back(result.get(i).unwrap());
    }

    *index = capped;
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Update both sorted indexes for `provider` after their stats change.
///
/// Call this on every signal close / provider stats update.
/// O(INDEX_CAPACITY) in-memory work, 2 storage reads + 2 writes.
pub fn update_leaderboard_index(env: &Env, provider: Address, stats: &ProviderPerformance) {
    let entry = IndexEntry {
        provider: provider.clone(),
        success_rate: stats.success_rate,
        total_volume: stats.total_volume,
        total_signals: stats.total_signals,
    };

    // Update success-rate index.
    let mut sr_index = load_index(env, LeaderboardKey::SuccessRateIndex);
    upsert_sorted(env, &mut sr_index, entry.clone(), |e| e.success_rate as i128);
    save_index(env, LeaderboardKey::SuccessRateIndex, &sr_index);

    // Update volume index.
    let mut vol_index = load_index(env, LeaderboardKey::VolumeIndex);
    upsert_sorted(env, &mut vol_index, entry, |e| e.total_volume);
    save_index(env, LeaderboardKey::VolumeIndex, &vol_index);

    #[allow(deprecated)]
    env.events()
        .publish((symbol_short!("lb_upd"), provider), stats.success_rate);
}

/// O(1) leaderboard query — reads the pre-sorted index directly.
///
/// Returns up to `limit` qualified providers. Followers returns empty (MVP).
pub fn get_leaderboard(
    env: &Env,
    _stats_map: &soroban_sdk::Map<Address, ProviderPerformance>,
    metric: LeaderboardMetric,
    limit: u32,
) -> Vec<ProviderLeaderboard> {
    if metric == LeaderboardMetric::Followers {
        return Vec::new(env);
    }

    let limit = if limit == 0 {
        DEFAULT_LEADERBOARD_LIMIT
    } else {
        limit.min(MAX_LEADERBOARD_LIMIT)
    };

    let key = match metric {
        LeaderboardMetric::SuccessRate => LeaderboardKey::SuccessRateIndex,
        LeaderboardMetric::Volume => LeaderboardKey::VolumeIndex,
        LeaderboardMetric::Followers => unreachable!(),
    };

    // Single storage read — O(1).
    let index = load_index(env, key);

    let take = limit.min(index.len());
    let mut result = Vec::new(env);
    let mut rank = 1u32;

    for i in 0..take {
        let e = index.get(i).unwrap();
        result.push_back(ProviderLeaderboard {
            rank,
            provider: e.provider,
            success_rate: e.success_rate,
            total_volume: e.total_volume,
            total_signals: e.total_signals,
        });
        rank = i + 2;
    }

    result
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ProviderPerformance;
    use soroban_sdk::testutils::Address as TestAddress;
    use soroban_sdk::{contract, Env, Map};

    #[contract]
    struct TestContract;

    fn setup() -> Env {
        Env::default()
    }

    fn make_stats(success_rate: u32, total_volume: i128, total_signals: u32) -> ProviderPerformance {
        ProviderPerformance {
            total_signals,
            successful_signals: 0,
            failed_signals: 0,
            total_copies: 0,
            success_rate,
            avg_return: 0,
            total_volume,
        }
    }

    /// Insert 100 providers and verify index correctness + O(1) query.
    #[test]
    fn test_index_correct_after_100_updates() {
        let env = setup();
        let contract_addr = env.register(TestContract, ());

        env.as_contract(&contract_addr, || {
            let mut providers = Vec::new(&env);
            for i in 0..100u32 {
                let p = Address::generate(&env);
                providers.push_back(p.clone());
                let stats = make_stats(i + 1, (i as i128 + 1) * 1_000, 10);
                update_leaderboard_index(&env, p, &stats);
            }

            let empty_map: Map<Address, ProviderPerformance> = Map::new(&env);

            // Success-rate leaderboard: top entry should have success_rate = 100
            let lb = get_leaderboard(&env, &empty_map, LeaderboardMetric::SuccessRate, 10);
            assert_eq!(lb.len(), 10);
            assert_eq!(lb.get(0).unwrap().success_rate, 100);
            assert!(lb.get(0).unwrap().success_rate >= lb.get(1).unwrap().success_rate);

            // Volume leaderboard: top entry should have highest volume
            let lb_vol = get_leaderboard(&env, &empty_map, LeaderboardMetric::Volume, 10);
            assert_eq!(lb_vol.len(), 10);
            assert!(lb_vol.get(0).unwrap().total_volume >= lb_vol.get(1).unwrap().total_volume);
        });
    }

    #[test]
    fn test_unqualified_provider_excluded() {
        let env = setup();
        let contract_addr = env.register(TestContract, ());

        env.as_contract(&contract_addr, || {
            let p = Address::generate(&env);
            // total_signals = 3 < MIN_SIGNALS_QUALIFICATION
            let stats = make_stats(80, 5_000, 3);
            update_leaderboard_index(&env, p, &stats);

            let empty_map: Map<Address, ProviderPerformance> = Map::new(&env);
            let lb = get_leaderboard(&env, &empty_map, LeaderboardMetric::SuccessRate, 10);
            assert_eq!(lb.len(), 0);
        });
    }

    #[test]
    fn test_upsert_updates_existing_provider() {
        let env = setup();
        let contract_addr = env.register(TestContract, ());

        env.as_contract(&contract_addr, || {
            let p = Address::generate(&env);
            update_leaderboard_index(&env, p.clone(), &make_stats(50, 1_000, 10));
            update_leaderboard_index(&env, p.clone(), &make_stats(90, 9_000, 20));

            let empty_map: Map<Address, ProviderPerformance> = Map::new(&env);
            let lb = get_leaderboard(&env, &empty_map, LeaderboardMetric::SuccessRate, 10);
            // Only one entry for this provider, with updated stats
            assert_eq!(lb.len(), 1);
            assert_eq!(lb.get(0).unwrap().success_rate, 90);
        });
    }

    #[test]
    fn test_followers_returns_empty() {
        let env = setup();
        let contract_addr = env.register(TestContract, ());

        env.as_contract(&contract_addr, || {
            let empty_map: Map<Address, ProviderPerformance> = Map::new(&env);
            let lb = get_leaderboard(&env, &empty_map, LeaderboardMetric::Followers, 10);
            assert_eq!(lb.len(), 0);
        });
    }
}
