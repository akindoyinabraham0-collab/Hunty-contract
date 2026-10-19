extern crate std;

use crate::{
    storage::Storage,
    types::{Hunt, HuntStatus, LeaderboardVisibility, PlayerProgress, RewardConfig},
    HuntyCore,
};
use soroban_sdk::{testutils::Address as _, xdr::ToXdr, Address, Env, String, Symbol, Vec};

fn hunt(env: &Env) -> Hunt {
    let mut categories = Vec::new(env);
    // Make full Hunt deserialization substantial without exceeding the public
    // description limit. These fields are irrelevant to compact timestamps.
    for _ in 0..5 {
        categories.push_back(String::from_str(env, "progress-read-budget-category"));
    }
    Hunt {
        hunt_id: 1,
        creator: Address::generate(env),
        title: String::from_str(env, "Progress read regression"),
        description: String::from_str(env, &"d".repeat(2000)),
        categories,
        difficulty_rating: 0,
        difficulty_override: None,
        status: HuntStatus::Active,
        created_at: 0,
        activated_at: 1000,
        start_time: 0,
        end_time: 0,
        reward_config: RewardConfig::new(env, 0, false, None, 0, 0, 0, None),
        time_bonus_start_bps: None,
        time_bonus_min_bps: None,
        time_bonus_decay_secs: None,
        total_clues: 0,
        required_clues: 0,
        completed_count: 0,
        max_submissions_per_minute: 0,
        max_attempts_per_clue: 0,
        start_multiplier_bps: 10000,
        registration_deadline: 0,
        allow_partial_scoring: false,
        team_mode: false,
        default_points: 100,
        attempt_cooldown_secs: 0,
        max_players: 0,
        is_private: false,
        remaining_slots: 0,
        leaderboard_visibility: LeaderboardVisibility::Public,
        invite_code_hash: None,
    }
}

#[test]
fn cached_progress_read_uses_less_cpu_than_loading_the_full_hunt() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract = env.register(HuntyCore, (admin,));
    let player = Address::generate(&env);
    env.as_contract(&contract, || {
        let hunt = hunt(&env);
        Storage::save_hunt(&env, &hunt);
        let mut progress = PlayerProgress::new(&env, player.clone(), 1, 1042);
        progress.is_completed = true;
        progress.completed_at = 1084;
        Storage::save_player_progress(&env, &progress, hunt.activated_at);

        env.cost_estimate().budget().reset_unlimited();
        let cached = Storage::try_get_player_progress(&env, 1, &player)
            .unwrap()
            .unwrap();
        let cached_cpu = env.cost_estimate().budget().cpu_instruction_cost();
        assert_eq!(cached.started_at, 1042);
        assert_eq!(cached.completed_at, 1084);

        Storage::invalidate_hunt_cache(&env, 1);
        env.cost_estimate().budget().reset_unlimited();
        let uncached = Storage::try_get_player_progress(&env, 1, &player)
            .unwrap()
            .unwrap();
        let uncached_cpu = env.cost_estimate().budget().cpu_instruction_cost();
        assert_eq!(uncached.started_at, cached.started_at);
        assert_eq!(uncached.completed_at, cached.completed_at);
        assert!(Storage::has_hunt_cache(&env, 1));
        assert!(
            cached_cpu < uncached_cpu,
            "cached progress read must avoid full Hunt loading: cached={cached_cpu}, uncached={uncached_cpu}"
        );
    });
}

#[test]
fn progress_read_promotes_legacy_instance_hunt_even_with_a_cache() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract = env.register(HuntyCore, (admin,));
    let player = Address::generate(&env);
    env.as_contract(&contract, || {
        let hunt = hunt(&env);
        let hunt_key = (Symbol::new(&env, "HUNT"), 1u64);
        env.storage().instance().set(&hunt_key, &hunt);
        Storage::save_hunt_cache(&env, &hunt);
        let progress = PlayerProgress::new(&env, player.clone(), 1, 1042);
        Storage::save_player_progress(&env, &progress, hunt.activated_at);

        let loaded = Storage::try_get_player_progress(&env, 1, &player)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.started_at, 1042);
        assert!(Storage::has_persistent_hunt(&env, 1));
        assert!(!env.storage().instance().has(&hunt_key));
    });
}

#[test]
fn progress_read_preserves_xdr_encoding_and_missing_hunt_fallback() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract = env.register(HuntyCore, (admin,));
    let player = Address::generate(&env);
    env.as_contract(&contract, || {
        let progress = PlayerProgress::new(&env, player.clone(), 1, 42);
        // An orphaned cache must not replace the existing missing-hunt base 0.
        Storage::save_hunt_cache(&env, &hunt(&env));
        let key = Storage::progress_key(1, &player);
        env.storage()
            .persistent()
            .set(&key, &progress.to_stored(0).to_xdr(&env));
        let loaded = Storage::try_get_player_progress(&env, 1, &player)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.started_at, 42);
        assert!(Storage::try_get_player_progress(&env, 2, &player)
            .unwrap()
            .is_none());
    });
}
