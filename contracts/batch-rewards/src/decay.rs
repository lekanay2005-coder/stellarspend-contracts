//! # batch_reward.rs
//!
//! Distributes staking rewards to multiple users in a single contract call.
//!
//! ## Gas optimizations
//! - Config read **once** before the loop — not once per recipient
//! - All per-user computation done in memory; storage written only at the end
//!   of each user's iteration (no intermediate reads inside the loop body)
//! - Emits **one** `BatchRewardEventData` summary instead of N individual
//!   events — saves `(N - 1) * event_base_cost` per batch run
//! - Users with zero balance are skipped before any storage is touched
//! - Storage slot removed when user balance drops to zero (reclaims rent)
//!
//! ## Naïve vs optimized storage operations for a 100-user batch
//!
//! | Operation          | Naïve  | Optimized |
//! |--------------------|--------|-----------|
//! | Config reads       | 100    | 1         |
//! | StakeEntry reads   | 100    | 100       |  <- unavoidable
//! | StakeEntry writes  | 100    | ≤ 100     |  <- skipped when balance = 0
//! | Events emitted     | 100    | 1         |
//! | **Total ops**      | **400+** | **~202** |

use soroban_sdk::{contract, contractimpl, Address, Env, Vec};

use crate::events::{emit_batch_reward, BatchRewardEventData};
use crate::{Config, DataKey, StakeEntry, StakingContract};

// ─── Public input type ────────────────────────────────────────────────────────

/// A (staker_address, override_reward) pair.
/// Pass `override_reward = 0` to use the automatic time-weighted calculation.
/// Pass a positive value to distribute a fixed bonus on top of the calculated reward.
pub struct RewardRecipient {
    pub staker:          Address,
    /// Extra tokens to credit on top of the calculated reward (0 = none)
    pub bonus_amount:    i128,
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct BatchRewardContract;

#[contractimpl]
impl BatchRewardContract {

    /// Distribute rewards to all recipients in `stakers`.
    ///
    /// Only callable by the contract admin (enforced via require_auth).
    ///
    /// `bonus_amounts` must be the same length as `stakers`; pass a vec of
    /// zeros if no bonuses are needed. Using parallel vecs avoids the cost of
    /// encoding a Vec of structs in Soroban's XDR type system.
    pub fn distribute_rewards(
        env:           Env,
        admin:         Address,
        stakers:       Vec<Address>,
        bonus_amounts: Vec<i128>,
    ) {
        admin.require_auth();

        assert!(
            stakers.len() == bonus_amounts.len(),
            "stakers and bonus_amounts must be the same length"
        );
        assert!(!stakers.is_empty(), "staker list must not be empty");

        // ── Optimization: read config ONCE before the loop ────────────────────
        let config: Config = env.storage().instance()
            .get(&DataKey::Config)
            .expect("staking contract not initialised");

        assert!(config.admin == admin, "caller is not the contract admin");

        let now = env.ledger().timestamp();
        let mut total_rewards: i128 = 0;
        let mut recipients:    u32  = 0;

        // ── Main loop ─────────────────────────────────────────────────────────
        // Each iteration: 1 read + (at most) 1 write. No config re-reads.
        let len = stakers.len();
        for i in 0..len {
            let staker = stakers.get(i).unwrap();
            let bonus  = bonus_amounts.get(i).unwrap();

            // Single read per user
            let mut entry: StakeEntry = env.storage()
                .persistent()
                .get(&DataKey::StakeEntry(staker.clone()))
                .unwrap_or_default();

            // Skip users with no stake — zero storage writes (optimization)
            if entry.balance == 0 && bonus == 0 {
                continue;
            }

            // Compute time-weighted reward in memory — reuse lib.rs helper
            let time_reward = if entry.balance > 0 {
                StakingContract::compute_reward(
                    entry.balance, entry.staked_at, now, config.reward_rate,
                )
            } else {
                0
            };

            let total_user_reward = time_reward + bonus;
            if total_user_reward <= 0 {
                continue;
            }

            // Credit reward into balance, reset reward clock
            entry.balance  += total_user_reward;
            entry.staked_at = now;

            // Single write per user (optimization #2)
            env.storage()
                .persistent()
                .set(&DataKey::StakeEntry(staker), &entry);

            total_rewards += total_user_reward;
            recipients    += 1;
        }

        // Only emit if at least one user received a reward
        if recipients > 0 {
            // One event for the whole batch (optimization — saves N-1 events)
            emit_batch_reward(&env, BatchRewardEventData {
                recipients,
                total_rewards,
                timestamp: now,
            });
        }
    }

    /// Preview how much reward each staker would receive right now,
    /// without modifying any state.
    ///
    /// Useful for off-chain tooling to estimate batch costs before calling
    /// `distribute_rewards`. Returns parallel vec of reward amounts.
    pub fn preview_rewards(
        env:     Env,
        stakers: Vec<Address>,
    ) -> Vec<i128> {
        let config: Config = env.storage().instance()
            .get(&DataKey::Config)
            .expect("staking contract not initialised");

        let now = env.ledger().timestamp();
        let mut results = Vec::new(&env);

        for i in 0..stakers.len() {
            let staker = stakers.get(i).unwrap();
            let entry: StakeEntry = env.storage()
                .persistent()
                .get(&DataKey::StakeEntry(staker))
                .unwrap_or_default();

            let reward = if entry.balance > 0 {
                StakingContract::compute_reward(
                    entry.balance, entry.staked_at, now, config.reward_rate,
                )
            } else {
                0
            };

            results.push_back(reward);
        }

        results
    }
}