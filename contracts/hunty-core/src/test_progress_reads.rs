extern crate std;

use crate::{
    errors::HuntError,
    storage::Storage,
    types::{
        Hunt, HuntStatus, LeaderboardIndexEntry, LeaderboardVisibility, PlayerProgress, RewardConfig,
        StoredPlayerProgress,
    },
    HuntyCore,
};
use soroban_sdk::{
    testutils::Address as _, xdr::ToXdr, Address, Env, String, Symbol, TryFromVal, Val, Vec,
};

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
fn supplied_activation_progress_read_uses_less_cpu_than_original_full_hunt_read() {
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
        let supplied = Storage::try_get_player_progress_with_activated_at(
            &env,
            1,
            &player,
            hunt.activated_at,
        )
        .unwrap()
        .unwrap();
        let supplied_cpu = env.cost_estimate().budget().cpu_instruction_cost();
        assert_eq!(supplied.started_at, 1042);
        assert_eq!(supplied.completed_at, 1084);

        // Reproduce the original normal read path exactly: load the full Hunt
        // for its activation time, then decode the same compact progress. No
        // cache invalidation or refresh is included in the measured baseline.
        env.cost_estimate().budget().reset_unlimited();
        let key = Storage::progress_key(1, &player);
        let activated_at = Storage::get_hunt(&env, 1).unwrap().activated_at;
        let val: Val = env.storage().persistent().get(&key).unwrap();
        let stored = StoredPlayerProgress::try_from_val(&env, &val).unwrap();
        let original = PlayerProgress::from_stored(&env, stored, player.clone(), 1, activated_at);
        let original_cpu = env.cost_estimate().budget().cpu_instruction_cost();
        assert_eq!(original.started_at, supplied.started_at);
        assert_eq!(original.completed_at, supplied.completed_at);
        assert!(
            supplied_cpu < original_cpu,
            "supplied activation must avoid original Hunt loading cost: supplied={supplied_cpu}, original={original_cpu}"
        );
    });
}

#[test]
fn supplied_activation_read_needs_neither_hunt_nor_cache_and_supports_xdr() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract = env.register(HuntyCore, (admin,));
    let player = Address::generate(&env);
    env.as_contract(&contract, || {
        let mut progress = PlayerProgress::new(&env, player.clone(), 1, 1042);
        progress.is_completed = true;
        progress.completed_at = 1084;
        let key = Storage::progress_key(1, &player);
        env.storage()
            .persistent()
            .set(&key, &progress.to_stored(1000).to_xdr(&env));
        assert!(!Storage::has_persistent_hunt(&env, 1));
        assert!(!Storage::has_hunt_cache(&env, 1));

        let loaded =
            Storage::get_player_progress_or_error_with_activated_at(&env, 1, &player, 1000)
                .unwrap();
        assert_eq!(loaded.started_at, 1042);
        assert_eq!(loaded.completed_at, 1084);

        // Even an unreadable Hunt payload cannot affect supplied timestamps.
        let hunt_key = (Symbol::new(&env, "HUNT"), 1u64);
        env.storage().persistent().set(&hunt_key, &123u32);
        let loaded = Storage::get_player_progress_with_activated_at(&env, 1, &player, 1000)
            .unwrap();
        assert_eq!(loaded.started_at, 1042);
        assert_eq!(loaded.completed_at, 1084);

        assert!(matches!(
            Storage::get_player_progress_or_error_with_activated_at(&env, 2, &player, 1000),
            Err(HuntError::PlayerNotRegistered)
        ));
        env.storage().persistent().set(&key, &123u32);
        assert!(matches!(
            Storage::get_player_progress_or_error_with_activated_at(&env, 1, &player, 1000),
            Err(HuntError::CorruptPlayerProgress)
        ));
        assert!(Storage::get_player_progress_with_activated_at(&env, 1, &player, 1000).is_none());
    });
}

#[test]
fn bulk_progress_reads_resolve_activation_once_per_batch() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract = env.register(HuntyCore, (admin,));
    env.as_contract(&contract, || {
        let hunt = hunt(&env);
        Storage::save_hunt(&env, &hunt);
        let mut entries = Vec::new(&env);
        for _ in 0..3 {
            let player = Address::generate(&env);
            let mut progress = PlayerProgress::new(&env, player.clone(), 1, 1042);
            progress.is_completed = true;
            progress.completed_at = 1084;
            Storage::save_player_progress(&env, &progress, hunt.activated_at);
            entries.push_back(LeaderboardIndexEntry {
                player,
                score: 0,
                completed_at: 1084,
                is_completed: true,
            });
        }
        Storage::save_leaderboard_index(&env, 1, &entries);
        let hit_key = Symbol::new(&env, "CHIT");

        Storage::reset_cache_counters(&env);
        let players = Storage::get_hunt_players(&env, 1);
        assert_eq!(players.len(), 3);
        assert_eq!(players.get(0).unwrap().started_at, 1042);
        assert_eq!(env.storage().instance().get::<_, u64>(&hit_key), Some(1));

        Storage::reset_cache_counters(&env);
        let completed = Storage::get_completed_hunt_players(&env, 1, 2);
        assert_eq!(completed.len(), 2);
        assert_eq!(completed.get(0).unwrap().completed_at, 1084);
        assert_eq!(env.storage().instance().get::<_, u64>(&hit_key), Some(1));
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
