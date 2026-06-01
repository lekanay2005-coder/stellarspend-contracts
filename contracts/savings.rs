//! Batch contributions to multiple savings goals.
#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Map, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavingsGoal {
    pub id: u32,
    pub owner: Address,
    pub target_amount: i128,
    pub saved_amount: i128,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Goal(u32),
}

#[contract]
pub struct SavingsContract;

#[contractimpl]
impl SavingsContract {
    /// Create a savings goal with a target amount.
    pub fn create_goal(env: Env, owner: Address, goal_id: u32, target_amount: i128) -> Result<(), &'static str> {
        if target_amount <= 0 {
            return Err("invalid_target_amount");
        }
        if Self::load_goal(&env, goal_id).is_some() {
            return Err("goal_already_exists");
        }
        let goal = SavingsGoal {
            id: goal_id,
            owner,
            target_amount,
            saved_amount: 0,
            created_at: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&DataKey::Goal(goal_id), &goal);
        Ok(())
    }

    /// Batch contribute to multiple savings goals atomically.
    pub fn batch_contribute(env: Env, user: Address, goal_ids: Vec<u32>, amounts: Vec<i128>) -> Result<(), &'static str> {
        if goal_ids.len() != amounts.len() {
            return Err("goal_amount_mismatch");
        }

        let mut pending: Map<u32, i128> = Map::new(&env);

        for (i, goal_id) in goal_ids.iter().enumerate() {
            let amount = amounts[i];
            if amount <= 0 {
                return Err("invalid_contribution_amount");
            }
            let goal = Self::load_goal(&env, *goal_id).ok_or("invalid_goal_id")?;
            let current_pending: i128 = pending.get(*goal_id).unwrap_or(0);
            let new_pending = current_pending.checked_add(amount).ok_or("over_contribution")?;
            let projected_saved = goal.saved_amount.checked_add(new_pending).ok_or("over_contribution")?;
            if projected_saved > goal.target_amount {
                return Err("over_contribution");
            }
            pending.set(*goal_id, &new_pending);
        }

        for (i, goal_id) in goal_ids.iter().enumerate() {
            Self::contribute(&env, &user, *goal_id, amounts[i])?;
        }

        Ok(())
    }

    pub fn get_goal(env: Env, goal_id: u32) -> Option<SavingsGoal> {
        Self::load_goal(&env, goal_id)
    }

    fn load_goal(env: &Env, goal_id: u32) -> Option<SavingsGoal> {
        env.storage().persistent().get(&DataKey::Goal(goal_id))
    }

    fn contribute(env: &Env, user: &Address, goal_id: u32, amount: i128) -> Result<(), &'static str> {
        let mut goal = Self::load_goal(env, goal_id).ok_or("invalid_goal_id")?;
        if goal.owner != *user {
            return Err("invalid_goal_owner");
        }
        let new_saved = goal.saved_amount.checked_add(amount).ok_or("over_contribution")?;
        if new_saved > goal.target_amount {
            return Err("over_contribution");
        }
        goal.saved_amount = new_saved;
        env.storage().persistent().set(&DataKey::Goal(goal_id), &goal);

        if goal.saved_amount == goal.target_amount {
            env.events().publish(("milestone", user.clone()), (goal_id, amount));
        }

        Ok(())
    }
}
