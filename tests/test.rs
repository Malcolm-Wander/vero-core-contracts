#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};
use vero_core_contracts::VeroContractClient;

fn setup() -> (Env, Address, Address, VeroContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, vero_core_contracts::VeroContract);
    let client = VeroContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(admin.clone());
    let token_address = token_contract.address();
    client.initialize(&token_address, &100i128);
    (env, admin, token_address, client)
}

fn lock_for_guardian(env: &Env, token_address: &Address, client: &VeroContractClient<'static>, guardian: &Address, amount: i128) {
    let token_admin = soroban_sdk::token::StellarAssetClient::new(env, token_address);
    token_admin.mint(guardian, &amount);
    client.lock_tokens(guardian, &amount);
}

#[test]
fn test_add_guardian_and_register_task() {
    let (_env, admin, _token, client) = setup();
    let guardian = Address::generate(&_env);

    client.add_guardian(&admin, &guardian);
    client.register_task(&admin, &1u64);

    let task = client.get_task(&1u64).unwrap();
    assert_eq!(task.id, 1);
    assert_eq!(task.votes, 0);
    assert!(!task.is_done);
}

#[test]
fn test_three_votes_flips_is_done() {
    let (env, admin, token, client) = setup();

    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let g3 = Address::generate(&env);

    client.add_guardian(&admin, &g1);
    client.add_guardian(&admin, &g2);
    client.add_guardian(&admin, &g3);
    client.register_task(&admin, &42u64);

    lock_for_guardian(&env, &token, &client, &g1, 101);
    lock_for_guardian(&env, &token, &client, &g2, 101);
    lock_for_guardian(&env, &token, &client, &g3, 101);

    client.vote(&g1, &42u64);
    client.vote(&g2, &42u64);
    client.vote(&g3, &42u64);

    let task = client.get_task(&42u64).unwrap();
    assert_eq!(task.votes, 3);
    assert!(task.is_done);
}

#[test]
fn test_duplicate_vote_rejected() {
    let (env, admin, token, client) = setup();
    let g = Address::generate(&env);

    client.add_guardian(&admin, &g);
    client.register_task(&admin, &7u64);
    lock_for_guardian(&env, &token, &client, &g, 101);
    client.vote(&g, &7u64);

    let result = client.try_vote(&g, &7u64);
    assert!(result.is_err());
}

#[test]
fn test_non_guardian_vote_rejected() {
    let (env, admin, _token, client) = setup();
    let stranger = Address::generate(&env);

    client.register_task(&admin, &99u64);

    let result = client.try_vote(&stranger, &99u64);
    assert!(result.is_err());
}

// ─── Drips cross-contract integration tests ────────────────────────────

#[test]
fn test_reward_stream_rejected_for_unverified_task() {
    let (env, admin, _token, client) = setup();
    let contributor = Address::generate(&env);
    let drips_addr = Address::generate(&env);

    // Register but do NOT verify the task (no votes)
    client.register_task(&admin, &10u64);

    let result = client.try_start_reward_stream(&admin, &drips_addr, &contributor, &10u64);
    assert!(result.is_err(), "should reject stream for unverified task");
}

#[test]
fn test_reward_stream_rejected_for_nonexistent_task() {
    let (env, admin, _token, client) = setup();
    let contributor = Address::generate(&env);
    let drips_addr = Address::generate(&env);

    // Task 999 was never registered
    let result = client.try_start_reward_stream(&admin, &drips_addr, &contributor, &999u64);
    assert!(result.is_err(), "should reject stream for nonexistent task");
}

#[test]
fn test_reward_stream_duplicate_rejected() {
    let (env, admin, token, client) = setup();
    let contributor = Address::generate(&env);

    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let g3 = Address::generate(&env);

    client.add_guardian(&admin, &g1);
    client.add_guardian(&admin, &g2);
    client.add_guardian(&admin, &g3);
    client.register_task(&admin, &50u64);

    lock_for_guardian(&env, &token, &client, &g1, 101);
    lock_for_guardian(&env, &token, &client, &g2, 101);
    lock_for_guardian(&env, &token, &client, &g3, 101);

    client.vote(&g1, &50u64);
    client.vote(&g2, &50u64);
    client.vote(&g3, &50u64);

    // Deploy a mock Drips contract to receive the cross-contract call
    let drips_contract_id = env.register_contract(None, MockDripsContract);

    // First stream should succeed
    client.start_reward_stream(&admin, &drips_contract_id, &contributor, &50u64);

    // Second attempt for same task should fail
    let result =
        client.try_start_reward_stream(&admin, &drips_contract_id, &contributor, &50u64);
    assert!(result.is_err(), "should reject duplicate stream");
}

#[test]
fn test_reward_stream_stored_after_success() {
    let (env, admin, token, client) = setup();
    let contributor = Address::generate(&env);

    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let g3 = Address::generate(&env);

    client.add_guardian(&admin, &g1);
    client.add_guardian(&admin, &g2);
    client.add_guardian(&admin, &g3);
    client.register_task(&admin, &77u64);

    lock_for_guardian(&env, &token, &client, &g1, 101);
    lock_for_guardian(&env, &token, &client, &g2, 101);
    lock_for_guardian(&env, &token, &client, &g3, 101);

    client.vote(&g1, &77u64);
    client.vote(&g2, &77u64);
    client.vote(&g3, &77u64);

    let drips_contract_id = env.register_contract(None, MockDripsContract);

    client.start_reward_stream(&admin, &drips_contract_id, &contributor, &77u64);

    let stream = client.get_reward_stream(&77u64).unwrap();
    assert_eq!(stream.task_id, 77);
    assert_eq!(stream.contributor, contributor);
    assert!(stream.active);
}

// ─── Token Locking Tests ────────────────────────────────────────────────

#[test]
fn test_voting_fails_if_tokens_not_locked() {
    let (env, admin, _token, client) = setup();
    let g = Address::generate(&env);

    client.add_guardian(&admin, &g);
    client.register_task(&admin, &100u64);

    // Try voting without locking tokens
    let result = client.try_vote(&g, &100u64);
    assert!(result.is_err());
}

#[test]
fn test_voting_fails_if_locked_balance_leq_threshold() {
    let (env, admin, token, client) = setup();
    let g = Address::generate(&env);

    client.add_guardian(&admin, &g);
    client.register_task(&admin, &100u64);

    // Lock exactly threshold (100) tokens
    lock_for_guardian(&env, &token, &client, &g, 100);

    // Try voting (should fail because locked balance must be > threshold, i.e., > 100)
    let result = client.try_vote(&g, &100u64);
    assert!(result.is_err());

    // Lock 1 more token (total 101)
    lock_for_guardian(&env, &token, &client, &g, 1);

    // Try voting (should succeed)
    client.vote(&g, &100u64);
    let task = client.get_task(&100u64).unwrap();
    assert_eq!(task.votes, 1);
}

#[test]
fn test_resign_guardian_refunds_tokens() {
    let (env, admin, token, client) = setup();
    let g = Address::generate(&env);

    client.add_guardian(&admin, &g);
    lock_for_guardian(&env, &token, &client, &g, 200);

    // Resign guardian
    client.resign_guardian(&g);

    // Verify resignation
    assert!(!client.is_guardian(&g));

    // Verify token refund
    let token_client = soroban_sdk::token::Client::new(&env, &token);
    assert_eq!(token_client.balance(&g), 200);
}

#[test]
fn test_unlock_fails_while_guardian() {
    let (env, admin, token, client) = setup();
    let g = Address::generate(&env);

    client.add_guardian(&admin, &g);
    lock_for_guardian(&env, &token, &client, &g, 200);

    // Try unlocking while still guardian (should fail)
    let result = client.try_unlock_tokens(&g);
    assert!(result.is_err());
}

#[test]
fn test_unlock_succeeds_for_non_guardian() {
    let (env, _admin, token, client) = setup();
    let non_guardian = Address::generate(&env);

    // Lock tokens for non-guardian
    lock_for_guardian(&env, &token, &client, &non_guardian, 150);

    // Unlock (should succeed because they are not a guardian)
    client.unlock_tokens(&non_guardian);

    // Verify token refund
    let token_client = soroban_sdk::token::Client::new(&env, &token);
    assert_eq!(token_client.balance(&non_guardian), 150);
}

// ─── Mock Drips contract for test isolation ────────────────────────────

use soroban_sdk::{contract, contractimpl};

/// A minimal mock of the Drips protocol contract used in tests.
/// It accepts `start_stream` calls without side-effects so we can
/// validate the Vero contract's cross-contract call logic in isolation.
#[contract]
pub struct MockDripsContract;

#[contractimpl]
impl MockDripsContract {
    pub fn start_stream(
        _env: Env,
        _contributor: Address,
        _task_id: u64,
        _resolution_status: u32,
    ) {
        // Mock: accept the call silently
    }
}
