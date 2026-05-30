use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};

// ─── Budget Types ───────────────────────────────────────────────────────────

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

/// Maximum number of requests in a single batch for optimization.
pub const MAX_BATCH_SIZE: u32 = 100;

/// Minimum spending limit (1 XLM in stroops).
pub const MIN_SPENDING_LIMIT: i128 = 10_000_000;

/// Maximum spending limit (1 billion XLM in stroops).
pub const MAX_SPENDING_LIMIT: i128 = 1_000_000_000_000_000_000;

/// Minimum reset window in seconds (1 hour).
pub const MIN_RESET_WINDOW_SECONDS: u64 = 3_600;

/// Maximum reset window in seconds (90 days).
pub const MAX_RESET_WINDOW_SECONDS: u64 = 7_776_000;

/// Escalation levels for spending limit enforcement.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum EscalationLevel {
    /// Small spend — automatic approval
    Small,
    /// Medium spend — logged but automatically approved
    Medium,
    /// Large spend — requires admin approval
    Large,
}

/// Configuration for spending escalation rules.
#[derive(Clone, Debug)]
#[contracttype]
pub struct EscalationConfig {
    /// Threshold for small-to-medium escalation (in stroops)
    pub small_threshold: i128,
    /// Threshold for medium-to-large escalation (in stroops)
    pub medium_threshold: i128,
    /// Whether escalation rules are enabled
    pub enabled: bool,
}

/// Represents a spending limit request for a user.
#[derive(Clone, Debug)]
#[contracttype]
pub struct SpendingLimitRequest {
    /// User's address
    pub user: Address,
    /// Monthly spending limit amount (in stroops)
    pub monthly_limit: i128,
    /// Reset window in seconds (e.g., 86400 for daily)
    pub reset_window_seconds: u64,
    /// Optional spending category
    pub category: Option<BudgetCategory>,
}

/// Represents a configured spending limit for a user.
#[derive(Clone, Debug)]
#[contracttype]
pub struct SpendingLimit {
    /// User's address
    pub user: Address,
    /// Monthly spending limit amount (in stroops)
    pub monthly_limit: i128,
    /// Reset window in seconds
    pub reset_window_seconds: u64,
    /// Current spending tracked in this period
    pub current_spending: i128,
    /// Optional category for the limit
    pub category: Option<BudgetCategory>,
    /// When the limit was last updated (ledger timestamp)
    pub updated_at: u64,
    /// Whether the limit is active
    pub is_active: bool,
}

/// Result of processing a single limit update.
#[derive(Clone, Debug)]
#[contracttype]
pub enum LimitUpdateResult {
    Success(SpendingLimit),
    Failure(Address, u32), // user address, error code
}

/// Aggregated metrics for a batch of limit updates.
#[derive(Clone, Debug)]
#[contracttype]
pub struct BatchLimitMetrics {
    /// Total number of limit update requests
    pub total_requests: u32,
    /// Number of successful updates
    pub successful_updates: u32,
    /// Number of failed updates
    pub failed_updates: u32,
    /// Total value of all limits
    pub total_limits_value: i128,
    /// Average limit amount
    pub avg_limit_amount: i128,
    /// Batch processing timestamp
    pub processed_at: u64,
}

/// Result of batch limit updates.
#[derive(Clone, Debug)]
#[contracttype]
pub struct BatchLimitResult {
    /// Batch ID
    pub batch_id: u64,
    /// Total number of requests
    pub total_requests: u32,
    /// Number of successful updates
    pub successful: u32,
    /// Number of failed updates
    pub failed: u32,
    /// Individual update results
    pub results: Vec<LimitUpdateResult>,
    /// Aggregated metrics
    pub metrics: BatchLimitMetrics,
}

/// Storage keys for contract state.
#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    /// Admin address
    Admin,
    /// Last created batch ID
    LastBatchId,
    /// Total limits updated lifetime
    TotalLimitsUpdated,
    /// Total batches processed lifetime
    TotalBatchesProcessed,
    /// Stored spending limit by user address
    SpendingLimit(Address),
    /// Windowed spending tracking (user, window_id)
    WindowSpending(Address, u64),
    /// Monthly spending tracking (user, month_id)
    MonthlySpending(Address, u64),
    /// Escalation configuration
    EscalationConfig,
    /// Pending large-spend approvals (spender, amount, timestamp)
    PendingApproval(Address),
}

/// Error codes for limit validation and enforcement.
pub mod ErrorCode {
    /// Invalid limit amount (negative, zero, or out of bounds)
    pub const INVALID_LIMIT: u32 = 0;
    /// Invalid limit amount (negative or zero)
    pub const INVALID_LIMIT_AMOUNT: u32 = 0;
    /// Invalid user address
    pub const INVALID_USER_ADDRESS: u32 = 1;
    /// Invalid reset window
    pub const INVALID_RESET_WINDOW: u32 = 2;
    /// Limit not found
    pub const LIMIT_NOT_FOUND: u32 = 3;
    /// Large spend requires admin approval
    pub const ESCALATION_APPROVAL_REQUIRED: u32 = 4;
    /// Pending approval not found or expired
    pub const APPROVAL_NOT_FOUND: u32 = 5;
}

/// Spending category for budget classification.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum BudgetCategory {
    Food,
    Transport,
    Rent,
    Entertainment,
}

/// Budget status.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum BudgetStatus {
    Active,
    Paused,
}

/// Budget record.
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
