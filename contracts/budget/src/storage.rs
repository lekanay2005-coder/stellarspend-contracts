use soroban_sdk::{contracttype, Address, Env, Map, Symbol, Vec};

/// Maximum transfer history entries kept per user.
pub const MAX_TRANSFER_HISTORY: u32 = 100;

/// Time window (seconds) for rapid-spending detection.
pub const RAPID_SPEND_WINDOW_SECONDS: u64 = 60;

/// Number of spends within the window that triggers a freeze.
pub const RAPID_SPEND_THRESHOLD: u32 = 3;

/// Default automatic freeze duration (seconds) after suspicious activity.
pub const DEFAULT_FREEZE_DURATION_SECONDS: u64 = 3_600;

/// Budget category with limit and spent tracking.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CategoryBudget {
    pub name: Symbol,
    pub limit: i128,
    pub spent: i128,
}

/// User budget configuration with per-category envelopes.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserBudget {
    pub user: Address,
    pub categories: Map<Symbol, CategoryBudget>,
    pub last_updated: u64,
}

/// Record of a category-to-category transfer.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CategoryTransfer {
    pub transfer_id: u64,
    pub user: Address,
    pub from_category: Symbol,
    pub to_category: Symbol,
    pub amount: i128,
    pub timestamp: u64,
}

/// Freeze state for a user's budget after suspicious activity.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetFreeze {
    pub is_frozen: bool,
    pub frozen_at: u64,
    pub auto_unfreeze_at: u64,
}

/// Recent spend timestamps used for rapid-spending detection.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpendingWindow {
    pub timestamps: Vec<u64>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    UserBudget(Address),
    TransferCounter,
    UserTransfers(Address),
    Transfer(u64),
    BudgetFreeze(Address),
    SpendingWindow(Address),
    SuspiciousActivityCount,
    Budget(Address),
    BudgetAsset(Address, Address),
    UserAssets(Address),
    TotalAllocated,
    PendingDeletion(Address),
}

pub fn get_user_budget(env: &Env, user: &Address) -> Option<UserBudget> {
    env.storage()
        .persistent()
        .get(&DataKey::UserBudget(user.clone()))
}

pub fn set_user_budget(env: &Env, budget: &UserBudget) {
    env.storage()
        .persistent()
        .set(&DataKey::UserBudget(budget.user.clone()), budget);
}

pub fn get_category_available(category: &CategoryBudget) -> i128 {
    category.limit.saturating_sub(category.spent)
}

pub fn next_transfer_id(env: &Env) -> u64 {
    let id: u64 = env
        .storage()
        .instance()
        .get(&DataKey::TransferCounter)
        .unwrap_or(0)
        + 1;
    env.storage().instance().set(&DataKey::TransferCounter, &id);
    id
}

pub fn record_transfer(env: &Env, transfer: &CategoryTransfer) {
    env.storage()
        .persistent()
        .set(&DataKey::Transfer(transfer.transfer_id), transfer);

    let mut history: Vec<u64> = env
        .storage()
        .persistent()
        .get(&DataKey::UserTransfers(transfer.user.clone()))
        .unwrap_or_else(|| Vec::new(env));

    history.push_back(transfer.transfer_id);

    while history.len() > MAX_TRANSFER_HISTORY {
        let oldest = history.get(0).unwrap();
        env.storage()
            .persistent()
            .remove(&DataKey::Transfer(oldest));
        let mut trimmed = Vec::new(env);
        for i in 1..history.len() {
            trimmed.push_back(history.get(i).unwrap());
        }
        history = trimmed;
    }

    env.storage()
        .persistent()
        .set(&DataKey::UserTransfers(transfer.user.clone()), &history);
}

pub fn get_transfer(env: &Env, transfer_id: u64) -> Option<CategoryTransfer> {
    env.storage()
        .persistent()
        .get(&DataKey::Transfer(transfer_id))
}

pub fn get_user_transfers(env: &Env, user: &Address) -> Vec<CategoryTransfer> {
    let ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(&DataKey::UserTransfers(user.clone()))
        .unwrap_or_else(|| Vec::new(env));

    let mut transfers = Vec::new(env);
    for id in ids.iter() {
        if let Some(transfer) = get_transfer(env, id) {
            transfers.push_back(transfer);
        }
    }
    transfers
}

pub fn get_budget_freeze(env: &Env, user: &Address) -> Option<BudgetFreeze> {
    env.storage()
        .persistent()
        .get(&DataKey::BudgetFreeze(user.clone()))
}

pub fn set_budget_freeze(env: &Env, user: &Address, freeze: &BudgetFreeze) {
    env.storage()
        .persistent()
        .set(&DataKey::BudgetFreeze(user.clone()), freeze);
}

pub fn clear_budget_freeze(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::BudgetFreeze(user.clone()));
}

pub fn is_budget_frozen(env: &Env, user: &Address, now: u64) -> bool {
    match get_budget_freeze(env, user) {
        Some(freeze) if freeze.is_frozen => {
            if freeze.auto_unfreeze_at > 0 && now >= freeze.auto_unfreeze_at {
                clear_budget_freeze(env, user);
                false
            } else {
                true
            }
        }
        _ => false,
    }
}

pub fn record_spend_timestamp(env: &Env, user: &Address, timestamp: u64) -> u32 {
    let mut window: SpendingWindow = env
        .storage()
        .persistent()
        .get(&DataKey::SpendingWindow(user.clone()))
        .unwrap_or(SpendingWindow {
            timestamps: Vec::new(env),
        });

    let cutoff = timestamp.saturating_sub(RAPID_SPEND_WINDOW_SECONDS);
    let mut recent = Vec::new(env);
    for ts in window.timestamps.iter() {
        if ts >= cutoff {
            recent.push_back(ts);
        }
    }
    recent.push_back(timestamp);
    window.timestamps = recent.clone();

    env.storage()
        .persistent()
        .set(&DataKey::SpendingWindow(user.clone()), &window);

    recent.len()
}

pub fn increment_suspicious_count(env: &Env) -> u64 {
    let count: u64 = env
        .storage()
        .instance()
        .get(&DataKey::SuspiciousActivityCount)
        .unwrap_or(0)
        + 1;
    env.storage()
        .instance()
        .set(&DataKey::SuspiciousActivityCount, &count);
    count
}
