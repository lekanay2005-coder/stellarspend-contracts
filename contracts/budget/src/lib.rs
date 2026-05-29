//! # Budget Contract
//!
//! Manages per-user category budgets with category-to-category transfers,
//! transfer history, and suspicious spending protection.

#![no_std]

mod storage;
#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contractimpl, panic_with_error, symbol_short, Address, Env, Map, Symbol, Vec,
};

pub use storage::{
    BudgetFreeze, CategoryBudget, CategoryTransfer, DataKey, SpendingWindow, UserBudget,
    DEFAULT_FREEZE_DURATION_SECONDS, RAPID_SPEND_THRESHOLD, RAPID_SPEND_WINDOW_SECONDS,
};

use storage::{
    clear_budget_freeze, get_budget_freeze, get_category_available, get_transfer,
    get_user_budget, get_user_transfers, increment_suspicious_count, is_budget_frozen,
    next_transfer_id, record_spend_timestamp, record_transfer, set_budget_freeze,
    set_user_budget,
};

/// Error codes for the budget contract.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum BudgetError {
    NotInitialized = 1,
    Unauthorized = 2,
    InvalidAmount = 3,
    BudgetNotFound = 4,
    CategoryNotFound = 5,
    InsufficientBalance = 6,
    SameCategory = 7,
    BudgetFrozen = 8,
    SuspiciousActivity = 9,
}

impl From<BudgetError> for soroban_sdk::Error {
    fn from(e: BudgetError) -> Self {
        soroban_sdk::Error::from_contract_error(e as u32)
    }
}

/// Events emitted by the budget contract.
pub struct BudgetEvents;

impl BudgetEvents {
    pub fn category_budget_set(env: &Env, user: &Address, category: &Symbol, limit: i128) {
        env.events().publish(
            (symbol_short!("budget"), symbol_short!("cat_set"), category.clone()),
            (user.clone(), limit),
        );
    }

    pub fn category_transfer(
        env: &Env,
        user: &Address,
        from: &Symbol,
        to: &Symbol,
        amount: i128,
        transfer_id: u64,
    ) {
        env.events().publish(
            (symbol_short!("budget"), symbol_short!("transfer"), transfer_id),
            (user.clone(), from.clone(), to.clone(), amount),
        );
    }

    pub fn spend_recorded(
        env: &Env,
        user: &Address,
        category: &Symbol,
        amount: i128,
        remaining: i128,
    ) {
        env.events().publish(
            (symbol_short!("budget"), symbol_short!("spent"), category.clone()),
            (user.clone(), amount, remaining),
        );
    }

    pub fn budget_frozen(env: &Env, user: &Address, frozen_at: u64, auto_unfreeze_at: u64) {
        env.events().publish(
            (symbol_short!("budget"), symbol_short!("frozen")),
            (user.clone(), frozen_at, auto_unfreeze_at),
        );
    }

    pub fn budget_unfrozen(env: &Env, user: &Address, unfrozen_at: u64) {
        env.events().publish(
            (symbol_short!("budget"), symbol_short!("unfrozen")),
            (user.clone(), unfrozen_at),
        );
    }
}

#[contract]
pub struct BudgetContract;

#[contractimpl]
impl BudgetContract {
    /// Initializes the contract with an admin address.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::TransferCounter, &0u64);
        env.storage()
            .instance()
            .set(&DataKey::SuspiciousActivityCount, &0u64);
    }

    /// Sets or updates a category budget limit for a user.
    pub fn set_category_budget(
        env: Env,
        admin: Address,
        user: Address,
        category: Symbol,
        limit: i128,
    ) {
        admin.require_auth();
        Self::require_admin(&env, &admin);

        if limit < 0 {
            panic_with_error!(&env, BudgetError::InvalidAmount);
        }

        let now = env.ledger().timestamp();
        let mut budget = get_user_budget(&env, &user).unwrap_or(UserBudget {
            user: user.clone(),
            categories: Map::new(&env),
            last_updated: now,
        });

        let spent = budget
            .categories
            .get(category.clone())
            .map(|c| c.spent)
            .unwrap_or(0);

        budget.categories.set(
            category.clone(),
            CategoryBudget {
                name: category.clone(),
                limit,
                spent,
            },
        );
        budget.last_updated = now;
        set_user_budget(&env, &budget);

        BudgetEvents::category_budget_set(&env, &user, &category, limit);
    }

    /// Transfers unused funds from one category to another.
    pub fn transfer_between_categories(
        env: Env,
        user: Address,
        from_category: Symbol,
        to_category: Symbol,
        amount: i128,
    ) -> u64 {
        user.require_auth();
        Self::assert_not_frozen(&env, &user);

        if amount <= 0 {
            panic_with_error!(&env, BudgetError::InvalidAmount);
        }
        if from_category == to_category {
            panic_with_error!(&env, BudgetError::SameCategory);
        }

        let mut budget = get_user_budget(&env, &user).unwrap_or_else(|| {
            panic_with_error!(&env, BudgetError::BudgetNotFound);
        });

        let from = budget
            .categories
            .get(from_category.clone())
            .unwrap_or_else(|| panic_with_error!(&env, BudgetError::CategoryNotFound));
        let available = get_category_available(&from);
        if available < amount {
            panic_with_error!(&env, BudgetError::InsufficientBalance);
        }

        let to = budget
            .categories
            .get(to_category.clone())
            .unwrap_or_else(|| panic_with_error!(&env, BudgetError::CategoryNotFound));

        budget.categories.set(
            from_category.clone(),
            CategoryBudget {
                name: from_category.clone(),
                limit: from.limit - amount,
                spent: from.spent,
            },
        );
        budget.categories.set(
            to_category.clone(),
            CategoryBudget {
                name: to_category.clone(),
                limit: to.limit + amount,
                spent: to.spent,
            },
        );
        budget.last_updated = env.ledger().timestamp();
        set_user_budget(&env, &budget);

        let transfer_id = next_transfer_id(&env);
        let transfer = CategoryTransfer {
            transfer_id,
            user: user.clone(),
            from_category,
            to_category,
            amount,
            timestamp: budget.last_updated,
        };
        record_transfer(&env, &transfer);
        BudgetEvents::category_transfer(
            &env,
            &user,
            &transfer.from_category,
            &transfer.to_category,
            amount,
            transfer_id,
        );

        transfer_id
    }

    /// Records spending from a category and detects rapid repeated spending.
    pub fn spend_from_category(env: Env, user: Address, category: Symbol, amount: i128) -> i128 {
        user.require_auth();
        Self::assert_not_frozen(&env, &user);

        if amount <= 0 {
            panic_with_error!(&env, BudgetError::InvalidAmount);
        }

        let now = env.ledger().timestamp();
        let mut budget = get_user_budget(&env, &user).unwrap_or_else(|| {
            panic_with_error!(&env, BudgetError::BudgetNotFound);
        });

        let cat = budget
            .categories
            .get(category.clone())
            .unwrap_or_else(|| panic_with_error!(&env, BudgetError::CategoryNotFound));

        let available = get_category_available(&cat);
        if available < amount {
            panic_with_error!(&env, BudgetError::InsufficientBalance);
        }

        let updated = CategoryBudget {
            name: category.clone(),
            limit: cat.limit,
            spent: cat.spent + amount,
        };
        let remaining = get_category_available(&updated);

        budget.categories.set(category.clone(), updated);
        budget.last_updated = now;
        set_user_budget(&env, &budget);

        let recent_count = record_spend_timestamp(&env, &user, now);
        if recent_count >= RAPID_SPEND_THRESHOLD {
            Self::freeze_for_suspicious_activity(&env, &user, now);
        }

        BudgetEvents::spend_recorded(&env, &user, &category, amount, remaining);
        remaining
    }

    /// Manually unfreezes a user's budget. Callable by admin or the user.
    pub fn unfreeze_budget(env: Env, caller: Address, user: Address) {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        if caller != admin && caller != user {
            panic_with_error!(&env, BudgetError::Unauthorized);
        }

        if get_budget_freeze(&env, &user).is_some() {
            clear_budget_freeze(&env, &user);
            BudgetEvents::budget_unfrozen(&env, &user, env.ledger().timestamp());
        }
    }

    /// Returns remaining balance for a category (limit - spent).
    pub fn get_category_balance(env: Env, user: Address, category: Symbol) -> i128 {
        let budget = get_user_budget(&env, &user).unwrap_or_else(|| {
            panic_with_error!(&env, BudgetError::BudgetNotFound);
        });
        let cat = budget
            .categories
            .get(category)
            .unwrap_or_else(|| panic_with_error!(&env, BudgetError::CategoryNotFound));
        get_category_available(&cat)
    }

    /// Returns a user's full budget configuration.
    pub fn get_user_budget(env: Env, user: Address) -> UserBudget {
        get_user_budget(&env, &user).unwrap_or_else(|| {
            panic_with_error!(&env, BudgetError::BudgetNotFound);
        })
    }

    /// Returns a single transfer record by ID.
    pub fn get_transfer(env: Env, transfer_id: u64) -> CategoryTransfer {
        get_transfer(&env, transfer_id).unwrap_or_else(|| {
            panic_with_error!(&env, BudgetError::BudgetNotFound);
        })
    }

    /// Returns transfer history for a user (most recent retained entries).
    pub fn get_transfer_history(env: Env, user: Address) -> Vec<CategoryTransfer> {
        get_user_transfers(&env, &user)
    }

    /// Returns whether the user's budget is currently frozen.
    pub fn is_frozen(env: Env, user: Address) -> bool {
        is_budget_frozen(&env, &user, env.ledger().timestamp())
    }

    /// Returns the current freeze state, if any.
    pub fn get_freeze_state(env: Env, user: Address) -> Option<BudgetFreeze> {
        get_budget_freeze(&env, &user)
    }

    /// Returns total suspicious-activity freeze events recorded.
    pub fn get_suspicious_activity_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::SuspiciousActivityCount)
            .unwrap_or(0)
    }

    /// Returns the admin address.
    pub fn get_admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized")
    }

    fn freeze_for_suspicious_activity(env: &Env, user: &Address, now: u64) {
        let auto_unfreeze_at = now.saturating_add(DEFAULT_FREEZE_DURATION_SECONDS);
        set_budget_freeze(
            env,
            user,
            &BudgetFreeze {
                is_frozen: true,
                frozen_at: now,
                auto_unfreeze_at,
            },
        );
        increment_suspicious_count(env);
        BudgetEvents::budget_frozen(env, user, now, auto_unfreeze_at);
    }

    fn assert_not_frozen(env: &Env, user: &Address) {
        if is_budget_frozen(env, user, env.ledger().timestamp()) {
            panic_with_error!(env, BudgetError::BudgetFrozen);
        }
    }

    fn require_admin(env: &Env, caller: &Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        if *caller != admin {
            panic_with_error!(env, BudgetError::Unauthorized);
        }
    }
}
