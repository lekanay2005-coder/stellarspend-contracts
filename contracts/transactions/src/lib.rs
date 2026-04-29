use soroban_sdk::{contract, contractimpl, contracttype, Env};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transaction {
    pub id: u64,
    pub amount: i128,
    pub sender: soroban_sdk::Address,
    pub receiver: soroban_sdk::Address,
    pub timestamp: u64,
    pub export: bool,
}

#[contracttype]
pub enum DataKey {
    Transaction(u64),
}

#[contract]
pub struct TransactionContract;

#[contractimpl]
impl TransactionContract {

    pub fn create_transaction(
        env: Env,
        id: u64,
        amount: i128,
        sender: soroban_sdk::Address,
        receiver: soroban_sdk::Address,
        timestamp: u64,
    ) -> Transaction {
        let tx = Transaction {
            id,
            amount,
            sender,
            receiver,
            timestamp,
            export: false,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Transaction(id), &tx);

        tx
    }

    pub fn set_export_flag(env: Env, id: u64, export: bool) -> Transaction {
        let mut tx: Transaction = env
            .storage()
            .persistent()
            .get(&DataKey::Transaction(id))
            .expect("Transaction not found");

        tx.export = export;

        env.storage()
            .persistent()
            .set(&DataKey::Transaction(id), &tx);

        tx
    }

    pub fn get_transaction(env: Env, id: u64) -> Transaction {
        env.storage()
            .persistent()
            .get(&DataKey::Transaction(id))
            .expect("Transaction not found")
    }

    /// Get all transactions for a user, sorted by timestamp (descending)
    pub fn get_user_transactions_sorted(env: Env, user: Address) -> Vec<Transaction> {
        let mut transactions = get_user_transactions(&env, user);
        
        // Simple bubble sort for demonstration (on-chain sorting can be expensive)
        let n = transactions.len();
        if n > 1 {
            for i in 0..n {
                for j in 0..n - i - 1 {
                    let tx_j = transactions.get(j).unwrap();
                    let tx_next = transactions.get(j + 1).unwrap();
                    if tx_j.timestamp < tx_next.timestamp {
                        transactions.set(j, tx_next);
                        transactions.set(j + 1, tx_j);
                    }
                }
            }
        }
        transactions
    }
    
    /// Get the last (most recent) transaction for a user
    pub fn get_last_transaction(env: Env, user: Address) -> Option<Transaction> {
        get_last_transaction(&env, user)
    }
    
    /// Get the total number of transactions recorded in the contract
    pub fn get_total_transactions_count(env: Env) -> u64 {
        get_total_transactions_count(&env)
    }
    
    /// Get all transactions in the contract
    pub fn get_all_transactions(env: Env) -> Vec<Transaction> {
        get_all_transactions(&env)
    }

    /// Get the total income from all transactions
    pub fn get_total_income(env: Env) -> i128 {
        storage::get_total_income(&env)
    }
    
    /// Get a paginated subset of all transactions.
    ///
    /// - `offset`: number of transactions to skip (0-based)
    /// - `limit`:  maximum number of transactions to return (capped at 100)
    pub fn get_transactions_paginated(env: Env, offset: u32, limit: u32) -> Vec<Transaction> {
        get_transactions_paginated(&env, offset, limit)
    }
    
    /// Clear all transactions for a user (only user can perform this action)
    pub fn clear_user_transactions(env: Env, user: Address) -> bool {
        user.require_auth();
        
        let success = clear_user_transactions(&env, user.clone());
        
        if success {
            env.events().publish(
                (symbol_short!("tx"), symbol_short!("cleared")),
                user,
            );
        }
        
        success
    }
    
    /// Get the admin address
    pub fn get_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::Admin)
    }

        tx.export
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env};

    #[test]
    fn test_export_flag_defaults_false() {
        let env = Env::default();
        let contract_id = env.register_contract(None, TransactionContract);
        let client = TransactionContractClient::new(&env, &contract_id);

        let sender = Address::generate(&env);
        let receiver = Address::generate(&env);

        let tx = client.create_transaction(&1u64, &1000i128, &sender, &receiver, &1000u64);
        assert_eq!(tx.export, false);
    }

    #[test]
    fn test_set_export_flag_true() {
        let env = Env::default();
        let contract_id = env.register_contract(None, TransactionContract);
        let client = TransactionContractClient::new(&env, &contract_id);

        let sender = Address::generate(&env);
        let receiver = Address::generate(&env);

        client.create_transaction(&2u64, &500i128, &sender, &receiver, &2000u64);
        let updated_tx = client.set_export_flag(&2u64, &true);
        assert_eq!(updated_tx.export, true);
    }

    #[test]
    fn test_set_export_flag_false() {
        let env = Env::default();
        let contract_id = env.register_contract(None, TransactionContract);
        let client = TransactionContractClient::new(&env, &contract_id);

        let sender = Address::generate(&env);
        let receiver = Address::generate(&env);

        client.create_transaction(&3u64, &250i128, &sender, &receiver, &3000u64);
        client.set_export_flag(&3u64, &true);
        let updated_tx = client.set_export_flag(&3u64, &false);
        assert_eq!(updated_tx.export, false);
    }

    #[test]
    fn test_get_export_flag() {
        let env = Env::default();
        let contract_id = env.register_contract(None, TransactionContract);
        let client = TransactionContractClient::new(&env, &contract_id);

        let sender = Address::generate(&env);
        let receiver = Address::generate(&env);

        client.create_transaction(&4u64, &750i128, &sender, &receiver, &4000u64);
        client.set_export_flag(&4u64, &true);

        let flag = client.get_export_flag(&4u64);
        assert_eq!(flag, true);
    }
}
