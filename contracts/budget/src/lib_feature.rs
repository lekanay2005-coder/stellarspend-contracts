//! # Budget Contract
//!
//! A Soroban smart contract for managing user budgets with validation and event emission.
//!
//! ## Features
//!
//! - **Individual Budget Updates**: Update single user budgets
//! - **Validation**: Prevents negative or zero allocations
//! - **Event Emission**: Tracks budget updates
//! - **Atomic Operations**: Ensures reliable state changes
//! - **Delegated Management**: Owners can authorize managers with granular permissions
//!
#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Vec,
    panic_with_error};

/// Error codes for the budget contract.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum BudgetError {
    /// Contract not initialized
    NotInitialized = 1,
    /// Caller is not authorized
    Unauthorized = 2,
    /// Invalid budget amount (negative or zero)
    InvalidAmount = 3,
    /// User not found
    UserNotFound = 4,
    /// Caller is not a delegated manager for this owner
    NotDelegated = 5,
    /// Requested amount exceeds the manager's granted permission limit
    ExceedsPermission = 6,
    /// Delegation exists but has been revoked (inactive)
    DelegationNotActive = 7,
}

impl From<BudgetError> for soroban_sdk::Error {
    fn from(e: BudgetError) -> Self {
        soroban_sdk::Error::from_contract_error(e as u32)
    }
}

/// Budget record for a user.
#[derive(Clone, Debug)]
#[contracttype]
pub struct BudgetRecord {
    pub user: Address,
    pub amount: i128,
    pub last_updated: u64,
}

/// Permission record for a delegated budget manager.
///
/// An owner grants a manager the ability to update the owner's budget up to
/// `max_amount`. The owner retains ultimate control and can always call
/// `update_budget` directly via the admin path or revoke the delegation.
#[derive(Clone, Debug)]
#[contracttype]
pub struct DelegationPermission {
    /// Maximum budget amount the manager may set (must be > 0)
    pub max_amount: i128,
    /// Ledger sequence when the delegation was created
    pub created_at: u64,
    /// Whether the delegation is currently active
    pub is_active: bool,
}

/// Storage keys for the contract.
#[derive(Clone, Debug)]
#[contracttype]
pub enum DataKey {
    Admin,
    Budget(Address),
    TotalAllocated,
    /// Delegation: (owner, manager) -> DelegationPermission
    Delegation(Address, Address),
    /// List of active manager addresses for an owner
    OwnerDelegates(Address),
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
        env.storage().instance().set(&DataKey::TotalAllocated, &0i128);
    }

    /// Updates a single user's budget. Admin only.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `admin` - The admin address calling the function
    /// * `user` - The user address to update budget for
    /// * `amount` - The new budget amount (must be > 0)
    pub fn update_budget(env: Env, admin: Address, user: Address, amount: i128) {
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
            last_updated: current_time,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Budget(user.clone()), &record);
        env.storage()
            .instance()
            .set(&DataKey::TotalAllocated, &total_allocated);

        env.events().publish(
            (symbol_short!("budget"), symbol_short!("updated")),
            (user, amount, current_time),
        );
    }

    /// Retrieves the budget record for a specific user.
    pub fn get_budget(env: Env, user: Address) -> Option<BudgetRecord> {
        env.storage().persistent().get(&DataKey::Budget(user))
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

    // ─── Delegated Budget Management (#600) ────────────────────────────────────

    /// Grants a manager the ability to update the caller's (owner's) budget up to
    /// `max_amount`. The owner retains full control and can always call
    /// `update_budget` directly. A manager can only be granted by the owner
    /// themselves.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `owner` - The budget owner granting delegation
    /// * `manager` - The address being granted manager rights
    /// * `max_amount` - Maximum budget amount the manager may set (must be > 0)
    pub fn delegate_manager(env: Env, owner: Address, manager: Address, max_amount: i128) {
        owner.require_auth();

        if max_amount <= 0 {
            panic_with_error!(&env, BudgetError::InvalidAmount);
        }

        let perm = DelegationPermission {
            max_amount,
            created_at: env.ledger().sequence() as u64,
            is_active: true,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Delegation(owner.clone(), manager.clone()), &perm);

        // Track manager in owner's delegate list
        let mut delegates: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::OwnerDelegates(owner.clone()))
            .unwrap_or(Vec::new(&env));
        if !delegates.contains(&manager) {
            delegates.push_back(manager.clone());
            env.storage()
                .persistent()
                .set(&DataKey::OwnerDelegates(owner.clone()), &delegates);
        }

        env.events().publish(
            (symbol_short!("delegate"), symbol_short!("granted")),
            (owner, manager, max_amount),
        );
    }

    /// Revokes a manager's delegation. Owner only.
    ///
    /// After revocation, the manager can no longer call `delegated_update_budget`
    /// on behalf of this owner.
    pub fn revoke_manager(env: Env, owner: Address, manager: Address) {
        owner.require_auth();

        let key = DataKey::Delegation(owner.clone(), manager.clone());
        if let Some(mut perm) = env
            .storage()
            .persistent()
            .get::<DataKey, DelegationPermission>(&key)
        {
            perm.is_active = false;
            env.storage().persistent().set(&key, &perm);

            env.events().publish(
                (symbol_short!("delegate"), symbol_short!("revoked")),
                (owner, manager),
            );
        }
    }

    /// Returns the delegation permission for a specific owner+manager pair, if any.
    pub fn get_delegation(
        env: Env,
        owner: Address,
        manager: Address,
    ) -> Option<DelegationPermission> {
        env.storage()
            .persistent()
            .get(&DataKey::Delegation(owner, manager))
    }

    /// Returns all manager addresses that the owner has ever delegated to.
    pub fn get_owner_delegates(env: Env, owner: Address) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::OwnerDelegates(owner))
            .unwrap_or(Vec::new(&env))
    }

    /// Allows a delegated manager to update the owner's budget within the
    /// permission limit granted by the owner.
    ///
    /// Managers operate strictly within assigned permissions; the owner retains
    /// ultimate control and can update without restriction via `update_budget`.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `manager` - The delegated manager calling this function
    /// * `owner` - The budget owner on whose behalf the manager is acting
    /// * `amount` - The new budget amount (must be <= `max_amount` in permission)
    ///
    /// # Errors
    /// * `NotDelegated` - if no delegation exists from owner to manager
    /// * `DelegationNotActive` - if the delegation was revoked
    /// * `ExceedsPermission` - if amount exceeds the manager's granted max
    /// * `InvalidAmount` - if amount is zero or negative
    pub fn delegated_update_budget(
        env: Env,
        manager: Address,
        owner: Address,
        amount: i128,
    ) {
        manager.require_auth();

        if amount <= 0 {
            panic_with_error!(&env, BudgetError::InvalidAmount);
        }

        // Load and verify delegation
        let perm: DelegationPermission = match env
            .storage()
            .persistent()
            .get(&DataKey::Delegation(owner.clone(), manager.clone()))
        {
            Some(p) => p,
            None => panic_with_error!(&env, BudgetError::NotDelegated),
        };

        if !perm.is_active {
            panic_with_error!(&env, BudgetError::DelegationNotActive);
        }

        if amount > perm.max_amount {
            panic_with_error!(&env, BudgetError::ExceedsPermission);
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
            .get::<DataKey, BudgetRecord>(&DataKey::Budget(owner.clone()))
        {
            total_allocated = total_allocated.checked_sub(old_record.amount).unwrap_or(0);
        }

        total_allocated = total_allocated.checked_add(amount).unwrap_or(i128::MAX);

        let record = BudgetRecord {
            user: owner.clone(),
            amount,
            last_updated: current_time,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Budget(owner.clone()), &record);
        env.storage()
            .instance()
            .set(&DataKey::TotalAllocated, &total_allocated);

        env.events().publish(
            (symbol_short!("budget"), symbol_short!("delegated")),
            (owner, manager, amount, current_time),
        );
    }

    /// Internal helper to verify admin authority.
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

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env};

    fn setup_test_contract() -> (Env, Address, BudgetContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(BudgetContract, ());
        let client = BudgetContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.initialize(&admin);
        (env, admin, client)
    }

    #[test]
    fn test_initialize() {
        let (_, admin, client) = setup_test_contract();
        assert_eq!(client.get_admin(), admin);
        assert_eq!(client.get_total_allocated(), 0);
    }

    #[test]
    #[should_panic(expected = "Already initialized")]
    fn test_initialize_twice_fails() {
        let (env, _, client) = setup_test_contract();
        let new_admin = Address::generate(&env);
        client.initialize(&new_admin);
    }

    #[test]
    fn test_update_budget() {
        let (env, admin, client) = setup_test_contract();
        let user = Address::generate(&env);

        client.update_budget(&admin, &user, &1_000_i128);

        let record = client.get_budget(&user).unwrap();
        assert_eq!(record.amount, 1_000);
        assert_eq!(record.user, user);
    }

    #[test]
    fn test_total_allocated_tracks_updates() {
        let (env, admin, client) = setup_test_contract();
        let user1 = Address::generate(&env);
        let user2 = Address::generate(&env);

        client.update_budget(&admin, &user1, &500_i128);
        client.update_budget(&admin, &user2, &300_i128);

        assert_eq!(client.get_total_allocated(), 800);
    }

    // ─── Delegation tests (#600) ───────────────────────────────────────────────

    #[test]
    fn test_delegate_manager_grants_permission() {
        let (env, admin, client) = setup_test_contract();
        let owner = Address::generate(&env);
        let manager = Address::generate(&env);

        client.delegate_manager(&owner, &manager, &1_000_i128);

        let perm = client.get_delegation(&owner, &manager).unwrap();
        assert_eq!(perm.max_amount, 1_000);
        assert!(perm.is_active);
    }

    #[test]
    fn test_owner_delegates_list_updated() {
        let (env, _, client) = setup_test_contract();
        let owner = Address::generate(&env);
        let manager1 = Address::generate(&env);
        let manager2 = Address::generate(&env);

        client.delegate_manager(&owner, &manager1, &500_i128);
        client.delegate_manager(&owner, &manager2, &800_i128);

        let delegates = client.get_owner_delegates(&owner);
        assert_eq!(delegates.len(), 2);
        assert!(delegates.contains(&manager1));
        assert!(delegates.contains(&manager2));
    }

    #[test]
    fn test_delegated_update_budget_within_limit() {
        let (env, admin, client) = setup_test_contract();
        let owner = Address::generate(&env);
        let manager = Address::generate(&env);

        // Grant manager permission up to 500
        client.delegate_manager(&owner, &manager, &500_i128);

        // Manager sets owner's budget to 400 (within limit)
        client.delegated_update_budget(&manager, &owner, &400_i128);

        let record = client.get_budget(&owner).unwrap();
        assert_eq!(record.amount, 400);
        assert_eq!(record.user, owner);
    }

    #[test]
    #[should_panic]
    fn test_delegated_update_budget_exceeds_permission_panics() {
        let (env, _, client) = setup_test_contract();
        let owner = Address::generate(&env);
        let manager = Address::generate(&env);

        client.delegate_manager(&owner, &manager, &500_i128);

        // Amount 501 exceeds max_amount 500 — must panic
        client.delegated_update_budget(&manager, &owner, &501_i128);
    }

    #[test]
    #[should_panic]
    fn test_delegated_update_budget_without_delegation_panics() {
        let (env, _, client) = setup_test_contract();
        let owner = Address::generate(&env);
        let rogue = Address::generate(&env);

        // No delegation was ever granted to rogue
        client.delegated_update_budget(&rogue, &owner, &100_i128);
    }

    #[test]
    #[should_panic]
    fn test_delegated_update_budget_after_revoke_panics() {
        let (env, _, client) = setup_test_contract();
        let owner = Address::generate(&env);
        let manager = Address::generate(&env);

        client.delegate_manager(&owner, &manager, &500_i128);
        client.revoke_manager(&owner, &manager);

        // Delegation was revoked — must panic
        client.delegated_update_budget(&manager, &owner, &100_i128);
    }

    #[test]
    fn test_revoke_manager_marks_inactive() {
        let (env, _, client) = setup_test_contract();
        let owner = Address::generate(&env);
        let manager = Address::generate(&env);

        client.delegate_manager(&owner, &manager, &500_i128);
        assert!(client.get_delegation(&owner, &manager).unwrap().is_active);

        client.revoke_manager(&owner, &manager);
        assert!(!client.get_delegation(&owner, &manager).unwrap().is_active);
    }

    #[test]
    fn test_owner_retains_control_after_delegation() {
        let (env, admin, client) = setup_test_contract();
        let owner = Address::generate(&env);
        let manager = Address::generate(&env);

        // Grant manager limited permission
        client.delegate_manager(&owner, &manager, &100_i128);

        // Admin (ultimate control) can still set any amount, including above manager's limit
        client.update_budget(&admin, &owner, &999_999_i128);

        let record = client.get_budget(&owner).unwrap();
        assert_eq!(record.amount, 999_999);
    }

    #[test]
    fn test_delegate_duplicate_manager_is_idempotent() {
        let (env, _, client) = setup_test_contract();
        let owner = Address::generate(&env);
        let manager = Address::generate(&env);

        client.delegate_manager(&owner, &manager, &300_i128);
        client.delegate_manager(&owner, &manager, &300_i128);

        // Should only appear once in the delegates list
        assert_eq!(client.get_owner_delegates(&owner).len(), 1);
    }

    #[test]
    #[should_panic]
    fn test_delegate_zero_max_amount_panics() {
        let (env, _, client) = setup_test_contract();
        let owner = Address::generate(&env);
        let manager = Address::generate(&env);

        client.delegate_manager(&owner, &manager, &0_i128);
    }
}
