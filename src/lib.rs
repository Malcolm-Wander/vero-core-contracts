#![no_std]

mod drips;
mod guardian;
mod task;
mod types;
pub mod events;

use soroban_sdk::{contract, contractimpl, Address, Env};
use types::{ContractError, DataKey, RewardStream};

pub use guardian::{add_guardian, is_guardian};
pub use task::{get_task, register_task};
pub use drips::{get_reward_stream, start_drips_stream};

const VOTE_THRESHOLD: u32 = 3;

#[contract]
pub struct VeroContract;

#[contractimpl]
impl VeroContract {
    pub fn initialize(
        env: Env,
        token: Address,
        threshold: i128,
    ) -> Result<(), ContractError> {
        let token_key = DataKey::TokenAddress;
        if env.storage().instance().has(&token_key) {
            return Err(ContractError::AlreadyInitialized);
        }
        env.storage().instance().set(&token_key, &token);
        env.storage().instance().set(&DataKey::LockThreshold, &threshold);
        Ok(())
    }

    pub fn lock_tokens(
        env: Env,
        guardian: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        guardian.require_auth();

        let token_key = DataKey::TokenAddress;
        if !env.storage().instance().has(&token_key) {
            return Err(ContractError::NotInitialized);
        }
        let token_address: Address = env.storage().instance().get(&token_key).unwrap();

        let client = soroban_sdk::token::Client::new(&env, &token_address);
        client.transfer(&guardian, &env.current_contract_address(), &amount);

        let balance_key = DataKey::LockedBalance(guardian.clone());
        let current_balance: i128 = env.storage().instance().get(&balance_key).unwrap_or(0);
        env.storage().instance().set(&balance_key, &(current_balance + amount));

        Ok(())
    }

    pub fn resign_guardian(
        env: Env,
        guardian: Address,
    ) -> Result<(), ContractError> {
        guardian.require_auth();

        let token_key = DataKey::TokenAddress;
        if !env.storage().instance().has(&token_key) {
            return Err(ContractError::NotInitialized);
        }

        if !guardian::is_guardian(&env, &guardian) {
            return Err(ContractError::NotGuardian);
        }

        let key = DataKey::Guardian(guardian.clone());
        env.storage().instance().set(&key, &false);

        let balance_key = DataKey::LockedBalance(guardian.clone());
        let locked_balance: i128 = env.storage().instance().get(&balance_key).unwrap_or(0);
        if locked_balance > 0 {
            let token_address: Address = env.storage().instance().get(&token_key).unwrap();
            let client = soroban_sdk::token::Client::new(&env, &token_address);
            client.transfer(&env.current_contract_address(), &guardian, &locked_balance);
            env.storage().instance().set(&balance_key, &0i128);
        }

        Ok(())
    }

    pub fn unlock_tokens(
        env: Env,
        guardian: Address,
    ) -> Result<(), ContractError> {
        guardian.require_auth();

        let token_key = DataKey::TokenAddress;
        if !env.storage().instance().has(&token_key) {
            return Err(ContractError::NotInitialized);
        }

        if guardian::is_guardian(&env, &guardian) {
            return Err(ContractError::StillGuardian);
        }

        let balance_key = DataKey::LockedBalance(guardian.clone());
        let locked_balance: i128 = env.storage().instance().get(&balance_key).unwrap_or(0);
        if locked_balance > 0 {
            let token_address: Address = env.storage().instance().get(&token_key).unwrap();
            let client = soroban_sdk::token::Client::new(&env, &token_address);
            client.transfer(&env.current_contract_address(), &guardian, &locked_balance);
            env.storage().instance().set(&balance_key, &0i128);
        }

        Ok(())
    }

    pub fn add_guardian(env: Env, admin: Address, guardian: Address) {
        guardian::add_guardian(&env, admin, guardian);
    }

    pub fn is_guardian(env: Env, guardian: Address) -> bool {
        guardian::is_guardian(&env, &guardian)
    }

    pub fn register_task(
        env: Env,
        admin: Address,
        task_id: u64,
    ) -> Result<(), ContractError> {
        task::register_task(&env, admin, task_id)
    }

    pub fn vote(env: Env, guardian: Address, task_id: u64) -> Result<(), ContractError> {
        guardian.require_auth();

        if !guardian::is_guardian(&env, &guardian) {
            return Err(ContractError::NotAuthorized);
        }

        let token_key = DataKey::TokenAddress;
        if !env.storage().instance().has(&token_key) {
            return Err(ContractError::NotInitialized);
        }
        let threshold: i128 = env.storage().instance().get(&DataKey::LockThreshold).unwrap_or(0);
        let balance_key = DataKey::LockedBalance(guardian.clone());
        let locked_balance: i128 = env.storage().instance().get(&balance_key).unwrap_or(0);

        if locked_balance <= threshold {
            return Err(ContractError::InsufficientLockedBalance);
        }

        let voted_key = DataKey::Voted(task_id, guardian.clone());
        if env.storage().instance().has(&voted_key) {
            return Err(ContractError::DuplicateVote);
        }
        env.storage().instance().set(&voted_key, &true);

        let task_key = DataKey::Task(task_id);
        let mut t: types::Task = env
            .storage()
            .instance()
            .get(&task_key)
            .ok_or(ContractError::NotAuthorized)?;

        t.votes += 1;
        if t.votes >= VOTE_THRESHOLD {
            t.is_done = true;
        }
        env.storage().instance().set(&task_key, &t);
        Ok(())
    }

    pub fn get_task(env: Env, task_id: u64) -> Option<types::Task> {
        task::get_task(&env, task_id)
    }

    /// Initiates a reward stream via the Drips protocol for a verified task.
    ///
    /// The caller (admin) must be authorized. The task must already be marked
    /// `is_done` via guardian consensus before a stream can be started.
    ///
    /// # Arguments
    /// * `admin` - The admin address authorizing the stream.
    /// * `drips_address` - The on-chain address of the Drips protocol contract.
    /// * `contributor` - The contributor's address to receive the reward stream.
    /// * `task_id` - The verified task ID.
    pub fn start_reward_stream(
        env: Env,
        admin: Address,
        drips_address: Address,
        contributor: Address,
        task_id: u64,
    ) -> Result<(), ContractError> {
        admin.require_auth();

        let result =
            drips::start_drips_stream(&env, drips_address, contributor.clone(), task_id);

        match &result {
            Ok(()) => {
                events::emit_reward_stream_started(&env, task_id, &contributor);
            }
            Err(_) => {
                events::emit_reward_stream_failed(&env, task_id, &contributor);
            }
        }

        result
    }

    /// Returns the reward stream record for a given task, if one exists.
    pub fn get_reward_stream(env: Env, task_id: u64) -> Option<RewardStream> {
        drips::get_reward_stream(&env, task_id)
    }
}
