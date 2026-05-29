use soroban_sdk::contracttype;

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