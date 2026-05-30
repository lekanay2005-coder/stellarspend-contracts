//! # Budget Contract
//!
//! Manages per-user category budgets with category-to-category transfers,
//! transfer history, suspicious spending protection, multi-asset budgets,
//! and deletion cooldown.

#![no_std]

mod storage;
#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contractimpl, contracttype, panic_with_error, symbol_short, Address, Env, Map,
    Symbol, Vec,
};

pub use storage::{
    BudgetFreeze, CategoryBudget, CategoryTransfer, DataKey, SpendingWindow, UserBudget,
    DEFAULT_FREEZE_DURATION_SECONDS, RAPID_SPEND_THRESHOLD, RAPID_SPEND_WINDOW_SECONDS,
};

use storage::{
    clear_budget_freeze, get_budget_freeze, get_category_available, get_transfer,
    get_user_budget as load_user_budget, get_user_transfers, increment_suspicious_count,
    is_budget_frozen, next_transfer_id, record_spend_timestamp, record_transfer, set_budget_freeze,
    set_user_budget,
};

/// Deletion cooldown period in seconds (24 hours).
pub const DELETION_COOLDOWN_SECONDS: u64 = 86_400;

/// Error codes for the budget contract.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum BudgetError {
    NotInitialized = 1,
    Unauthorized = 2,
    InvalidAmount = 3,
    UserNotFound = 4,
    DeletionCooldownNotElapsed = 5,
    NoPendingDeletion = 6,
    BudgetNotFound = 7,
    CategoryNotFound = 8,
    InsufficientBalance = 9,
    SameCategory = 10,
    BudgetFrozen = 11,
    SuspiciousActivity = 12,
}

impl From<BudgetError> for soroban_sdk::Error {
    fn from(e: BudgetError) -> Self {
        soroban_sdk::Error::from_contract_error(e as u32)
    }
}

/// Budget record for a user with multi-asset support.
#[derive(Clone, Debug)]
#[contracttype]
pub struct BudgetRecord {
    pub user: Address,
    pub amount: i128,
    pub asset: Option<Address>,
    pub last_updated: u64,
}

/// Pending deletion record with cooldown expiry timestamp.
#[derive(Clone, Debug)]
#[contracttype]
pub struct PendingDeletion {
    pub user: Address,
    pub cooldown_expiry: u64,
}

/// Events emitted by the budget contract.
pub struct BudgetEvents;

impl BudgetEvents {
    pub fn category_budget_set(env: &Env, user: &Address, category: &Symbol, limit: i128) {
        env.events().publish(
            (
                symbol_short!("budget"),
                symbol_short!("cat_set"),
                category.clone(),
            ),
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
            (
                symbol_short!("budget"),
                symbol_short!("transfer"),
                transfer_id,
            ),
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
            (
                symbol_short!("budget"),
                symbol_short!("spent"),
                category.clone(),
            ),
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
        env.storage()
            .instance()
            .set(&DataKey::TransferCounter, &0u64);
        env.storage()
            .instance()
            .set(&DataKey::SuspiciousActivityCount, &0u64);
        env.storage()
            .instance()
            .set(&DataKey::TotalAllocated, &0i128);
    }

    /// Updates a single user's budget with optional multi-asset support.
    pub fn update_budget(
        env: Env,
        admin: Address,
        user: Address,
        amount: i128,
        asset: Option<Address>,
    ) {
        admin.require_auth();
        Self::require_admin(&env, &admin);

        if amount <= 0 {
            panic_with_error!(&env, BudgetError::InvalidAmount);
        }

        let current_time = env.ledger().timestamp();
        let mut total_allocated: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalAllocated)
            .unwrap_or(0);

        if let Some(old_record) = env
            .storage()
            .persistent()
            .get::<DataKey, BudgetRecord>(&DataKey::Budget(user.clone()))
        {
            total_allocated = total_allocated.checked_sub(old_record.amount).unwrap_or(0);
        }

        total_allocated = total_allocated.checked_add(amount).unwrap_or(i128::MAX);

        let record = BudgetRecord {
            user: user.clone(),
            amount,
            asset: asset.clone(),
            last_updated: current_time,
        };

        if let Some(ref asset_addr) = asset {
            env.storage().persistent().set(
                &DataKey::BudgetAsset(user.clone(), asset_addr.clone()),
                &record,
            );
            let mut user_assets: Vec<Address> = env
                .storage()
                .persistent()
                .get(&DataKey::UserAssets(user.clone()))
                .unwrap_or(Vec::new(&env));
            if !user_assets.contains(asset_addr) {
                user_assets.push_back(asset_addr.clone());
                env.storage()
                    .persistent()
                    .set(&DataKey::UserAssets(user.clone()), &user_assets);
            }
        } else {
            env.storage()
                .persistent()
                .set(&DataKey::Budget(user.clone()), &record);
        }

        env.storage()
            .instance()
            .set(&DataKey::TotalAllocated, &total_allocated);

        env.events().publish(
            (symbol_short!("budget"), symbol_short!("updated")),
            (user, amount, current_time),
        );
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
        let mut budget = load_user_budget(&env, &user).unwrap_or(UserBudget {
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

        let mut budget = load_user_budget(&env, &user).unwrap_or_else(|| {
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
        let mut budget = load_user_budget(&env, &user).unwrap_or_else(|| {
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

    /// Schedules a budget for deletion with a 24-hour cooldown.
    pub fn schedule_deletion(env: Env, admin: Address, user: Address) {
        admin.require_auth();
        Self::require_admin(&env, &admin);

        if env
            .storage()
            .persistent()
            .get::<DataKey, BudgetRecord>(&DataKey::Budget(user.clone()))
            .is_none()
        {
            panic_with_error!(&env, BudgetError::UserNotFound);
        }

        let current_time = env.ledger().timestamp();
        let cooldown_expiry = current_time
            .checked_add(DELETION_COOLDOWN_SECONDS)
            .unwrap_or(u64::MAX);

        let pending = PendingDeletion {
            user: user.clone(),
            cooldown_expiry,
        };

        env.storage()
            .persistent()
            .set(&DataKey::PendingDeletion(user.clone()), &pending);

        env.events().publish(
            (symbol_short!("budget"), symbol_short!("del_sched")),
            (user, cooldown_expiry),
        );
    }

    /// Cancels a pending budget deletion.
    pub fn cancel_deletion(env: Env, admin: Address, user: Address) {
        admin.require_auth();
        Self::require_admin(&env, &admin);

        if !env
            .storage()
            .persistent()
            .has(&DataKey::PendingDeletion(user.clone()))
        {
            panic_with_error!(&env, BudgetError::NoPendingDeletion);
        }

        env.storage()
            .persistent()
            .remove(&DataKey::PendingDeletion(user.clone()));

        env.events()
            .publish((symbol_short!("budget"), symbol_short!("del_canc")), user);
    }

    /// Executes a scheduled budget deletion after the cooldown period has elapsed.
    pub fn execute_deletion(env: Env, admin: Address, user: Address) {
        admin.require_auth();
        Self::require_admin(&env, &admin);

        let pending: PendingDeletion = env
            .storage()
            .persistent()
            .get(&DataKey::PendingDeletion(user.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, BudgetError::NoPendingDeletion));

        let current_time = env.ledger().timestamp();
        if current_time < pending.cooldown_expiry {
            panic_with_error!(&env, BudgetError::DeletionCooldownNotElapsed);
        }

        let mut old_amount = env
            .storage()
            .persistent()
            .get::<DataKey, BudgetRecord>(&DataKey::Budget(user.clone()))
            .map(|r| r.amount)
            .unwrap_or(0);

        let user_assets: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::UserAssets(user.clone()))
            .unwrap_or(Vec::new(&env));
        for asset in user_assets.iter() {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<DataKey, BudgetRecord>(&DataKey::BudgetAsset(user.clone(), asset.clone()))
            {
                old_amount = old_amount.checked_add(record.amount).unwrap_or(old_amount);
            }
            env.storage()
                .persistent()
                .remove(&DataKey::BudgetAsset(user.clone(), asset.clone()));
        }
        env.storage()
            .persistent()
            .remove(&DataKey::UserAssets(user.clone()));

        env.storage()
            .persistent()
            .remove(&DataKey::Budget(user.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::PendingDeletion(user.clone()));

        let total_allocated: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalAllocated)
            .unwrap_or(0);
        let new_total = total_allocated.checked_sub(old_amount).unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalAllocated, &new_total);

        env.events().publish(
            (symbol_short!("budget"), symbol_short!("deleted")),
            (user, current_time),
        );
    }

    /// Returns remaining balance for a category (limit - spent).
    pub fn get_category_balance(env: Env, user: Address, category: Symbol) -> i128 {
        let budget = load_user_budget(&env, &user).unwrap_or_else(|| {
            panic_with_error!(&env, BudgetError::BudgetNotFound);
        });
        let cat = budget
            .categories
            .get(category)
            .unwrap_or_else(|| panic_with_error!(&env, BudgetError::CategoryNotFound));
        get_category_available(&cat)
    }

    /// Returns a user's full category budget configuration.
    pub fn get_user_budget(env: Env, user: Address) -> UserBudget {
        load_user_budget(&env, &user).unwrap_or_else(|| {
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

    /// Returns the pending deletion for a user, if one exists.
    pub fn get_pending_deletion(env: Env, user: Address) -> Option<PendingDeletion> {
        env.storage()
            .persistent()
            .get(&DataKey::PendingDeletion(user))
    }

    /// Retrieves the budget for a specific user (default/native asset).
    pub fn get_budget(env: Env, user: Address) -> Option<BudgetRecord> {
        env.storage().persistent().get(&DataKey::Budget(user))
    }

    /// Retrieves the budget for a specific user and asset.
    pub fn get_budget_by_asset(env: Env, user: Address, asset: Address) -> Option<BudgetRecord> {
        env.storage()
            .persistent()
            .get(&DataKey::BudgetAsset(user, asset))
    }

    /// Returns all asset contract IDs for a user's multi-asset budgets.
    pub fn get_user_assets(env: Env, user: Address) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::UserAssets(user))
            .unwrap_or(Vec::new(&env))
    }

    /// Returns the admin address.
    pub fn get_admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized")
    }

    /// Returns the total allocated budget amount.
    pub fn get_total_allocated(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalAllocated)
            .unwrap_or(0)
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
