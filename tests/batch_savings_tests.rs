// Integration tests for batch contributions to savings goals.

use super::*;
use soroban_sdk::{testutils::Address as _, Address, Env};

#[test]
fn test_batch_contribute_success() {
    let env = Env::default();
    let user = Address::generate(&env);
    let goal_ids = vec![1, 2, 3];
    let targets = vec![100, 200, 300];
    let amounts = vec![100, 200, 300];

    for (goal_id, target) in goal_ids.iter().zip(targets.iter()) {
        assert_eq!(
            SavingsContract::create_goal(env.clone(), user.clone(), *goal_id, *target),
            Ok(())
        );
    }

    let result = SavingsContract::batch_contribute(
        env.clone(),
        user.clone(),
        goal_ids.clone(),
        amounts.clone(),
    );
    assert!(result.is_ok());

    for (i, goal_id) in goal_ids.iter().enumerate() {
        let goal = SavingsContract::get_goal(env.clone(), *goal_id).expect("goal exists");
        assert_eq!(goal.saved_amount, amounts[i]);
    }

    let events = env.events().all();
    assert!(events.iter().any(|e| e.topics.0 == "milestone"));
}

#[test]
fn test_goal_amount_mismatch() {
    let env = Env::default();
    let user = Address::generate(&env);
    let goal_ids = vec![1, 2];
    let amounts = vec![100];
    let result = SavingsContract::batch_contribute(
        env.clone(),
        user.clone(),
        goal_ids.clone(),
        amounts.clone(),
    );
    assert_eq!(result, Err("goal_amount_mismatch"));
}

#[test]
fn test_invalid_goal_id() {
    let env = Env::default();
    let user = Address::generate(&env);
    let goal_ids = vec![999];
    let amounts = vec![100];
    let result = SavingsContract::batch_contribute(env.clone(), user.clone(), goal_ids, amounts);
    assert_eq!(result, Err("invalid_goal_id"));
}

#[test]
fn test_over_contribution() {
    let env = Env::default();
    let user = Address::generate(&env);
    assert_eq!(SavingsContract::create_goal(env.clone(), user.clone(), 1, 500), Ok(()));

    let goal_ids = vec![1];
    let amounts = vec![600];
    let result = SavingsContract::batch_contribute(env.clone(), user.clone(), goal_ids, amounts);
    assert_eq!(result, Err("over_contribution"));
}
