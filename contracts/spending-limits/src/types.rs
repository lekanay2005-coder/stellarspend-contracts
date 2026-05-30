use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};

// ─── Budget Types ───────────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum BudgetStatus {
    Active,
    Paused,
}

#[derive(Clone)]
#[contracttype]
pub struct Budget {
    pub owner: Address,
    pub limit: i128,
    pub spent: i128,
    pub status: BudgetStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum BudgetCategory {
    Food,
    Transport,
    Rent,
    Entertainment,
}

// ─── Constants ──────────────────────────────────────────────────────────────

pub const MAX_BATCH_SIZE: u32 = 100;
pub const MIN_SPENDING_LIMIT: i128 = 1_000_000;
pub const MAX_SPENDING_LIMIT: i128 = 100_000_000_000_000_000;
pub const MIN_RESET_WINDOW_SECONDS: u64 = 86_400;
pub const MAX_RESET_WINDOW_SECONDS: u64 = 86_400 * 365;

// ─── Storage Keys ───────────────────────────────────────────────────────────

/// Storage keys for the spending limits contract.
///
/// # Storage Optimization (Issue #484)
///
/// The previously separate `Admin`, `LastBatchId`, `TotalLimitsUpdated`, and
/// `TotalBatchesProcessed` keys have been consolidated into a single
/// `LimitsConfig` struct. This reduces instance storage operations from 4
/// reads/writes to 1 per access, lowering the overall storage footprint and
/// Soroban rent costs.
#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    /// Consolidated limits configuration (was 4 separate keys).
    LimitsConfig,
    /// Per-user spending limit.
    SpendingLimit(Address),
    /// Window-level spending counter.
    WindowSpending(Address, u64),
    /// Month-level spending counter.
    MonthlySpending(Address, u64),
}

// ─── Limits Config ──────────────────────────────────────────────────────────

/// Consolidated instance-storage configuration for the spending limits
/// contract.
///
/// Replaces the four previously separate storage entries:
///   `Admin`, `LastBatchId`, `TotalLimitsUpdated`, `TotalBatchesProcessed`.
///
/// Reading/writing a single struct is ~4× more efficient than reading/writing
/// four individual keys due to reduced storage I/O overhead in Soroban.
#[derive(Clone)]
#[contracttype]
pub struct LimitsConfig {
    pub admin: Address,
    pub last_batch_id: u64,
    pub total_limits_updated: u64,
    pub total_batches_processed: u64,
}

// ─── Spending Limit Types ───────────────────────────────────────────────────

#[derive(Clone)]
#[contracttype]
pub struct SpendingLimit {
    pub user: Address,
    pub monthly_limit: i128,
    pub reset_window_seconds: u64,
    pub current_spending: i128,
    pub category: Option<Symbol>,
    pub updated_at: u64,
    pub is_active: bool,
}

#[derive(Clone)]
#[contracttype]
pub struct SpendingLimitRequest {
    pub user: Address,
    pub monthly_limit: i128,
    pub reset_window_seconds: u64,
    pub category: Option<Symbol>,
}

// ─── Result Types ───────────────────────────────────────────────────────────

#[derive(Clone)]
#[contracttype]
pub enum LimitUpdateResult {
    Success(SpendingLimit),
    Failure(Address, u32),
}

#[derive(Clone)]
#[contracttype]
pub struct BatchLimitMetrics {
    pub total_requests: u32,
    pub successful_updates: u32,
    pub failed_updates: u32,
    pub total_limits_value: i128,
    pub avg_limit_amount: i128,
    pub processed_at: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct BatchLimitResult {
    pub batch_id: u64,
    pub total_requests: u32,
    pub successful: u32,
    pub failed: u32,
    pub results: Vec<LimitUpdateResult>,
    pub metrics: BatchLimitMetrics,
}

// ─── Error Codes ────────────────────────────────────────────────────────────

pub struct ErrorCode;

impl ErrorCode {
    pub const INVALID_USER_ADDRESS: u32 = 1;
    pub const INVALID_LIMIT: u32 = 2;
}

// ─── Event Helpers ──────────────────────────────────────────────────────────

pub struct LimitEvents;

impl LimitEvents {
    pub fn batch_started(env: &Env, batch_id: u64, count: u32) {
        env.events().publish(
            (Symbol::new(env, "limit"), Symbol::new(env, "batch_started")),
            (batch_id, count),
        );
    }

    pub fn limit_updated(env: &Env, batch_id: u64, limit: &SpendingLimit) {
        env.events().publish(
            (Symbol::new(env, "limit"), Symbol::new(env, "updated")),
            (batch_id, limit.user.clone(), limit.monthly_limit),
        );
    }

    pub fn high_value_limit(env: &Env, batch_id: u64, user: &Address, amount: i128) {
        env.events().publish(
            (Symbol::new(env, "limit"), Symbol::new(env, "high_value")),
            (batch_id, user.clone(), amount),
        );
    }

    pub fn limit_update_failed(env: &Env, batch_id: u64, user: &Address, error_code: u32) {
        env.events().publish(
            (Symbol::new(env, "limit"), Symbol::new(env, "update_failed")),
            (batch_id, user.clone(), error_code),
        );
    }

    pub fn batch_completed(env: &Env, batch_id: u64, success: u32, failed: u32, total: i128) {
        env.events().publish(
            (
                Symbol::new(env, "limit"),
                Symbol::new(env, "batch_completed"),
            ),
            (batch_id, success, failed, total),
        );
    }

    pub fn limit_exceeded(
        env: &Env,
        user: &Address,
        amount: i128,
        remaining_window: i128,
        remaining_monthly: i128,
    ) {
        env.events().publish(
            (Symbol::new(env, "limit"), Symbol::new(env, "exceeded")),
            (user.clone(), amount, remaining_window, remaining_monthly),
        );
    }
}
