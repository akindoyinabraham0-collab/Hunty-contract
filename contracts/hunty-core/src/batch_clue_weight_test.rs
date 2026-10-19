//! #1252: `add_clues` used to pass `None` for the clue weight, so every
//! batch-added clue had weight 1 and a batch hunt could not be configured the
//! same way as a hunt built with `add_clue`. `BatchClueInput` now carries the
//! same optional weight as `add_clue`.

use crate::errors::HuntErrorCode;
use crate::storage::Storage;
use crate::types::{BatchClueInput, HuntStatus, PlayerProgress};
use crate::HuntyCore;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Env, String};

fn create_hunt(env: &Env, contract_id: &Address, creator: &Address, title: &str) -> u64 {
    env.as_contract(contract_id, || {
        HuntyCore::create_hunt(
            env.clone(),
            creator.clone(),
            String::from_str(env, title),
            String::from_str(env, "Batch clue weight"),
            None,
            None,
            0,
            None,
            None,
        )
        .unwrap()
    })
}

fn batch_clue(env: &Env, question: &str, difficulty: u32, weight: Option<u32>) -> BatchClueInput {
    BatchClueInput {
        question: String::from_str(env, question),
        answer: String::from_str(env, "answer"),
        points: 10,
        is_required: true,
        difficulty,
        weight,
    }
}

#[test]
fn batched_clues_keep_their_configured_weight() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(HuntyCore, ());
    let creator = Address::generate(&env);
    let hunt_id = create_hunt(&env, &contract_id, &creator, "Weighted Batch");

    let clues = vec![
        &env,
        batch_clue(&env, "Q1", 2, Some(3)),
        batch_clue(&env, "Q2", 2, None),
        batch_clue(&env, "Q3", 4, Some(1)),
    ];
    env.as_contract(&contract_id, || {
        HuntyCore::add_clues(env.clone(), hunt_id, clues.clone()).unwrap();
    });

    env.as_contract(&contract_id, || {
        assert_eq!(
            Storage::get_clue_or_error(&env, hunt_id, 1).unwrap().weight,
            3
        );
        // No weight keeps the documented default of 1.
        assert_eq!(
            Storage::get_clue_or_error(&env, hunt_id, 2).unwrap().weight,
            1
        );
        assert_eq!(
            Storage::get_clue_or_error(&env, hunt_id, 3).unwrap().weight,
            1
        );
    });
}

#[test]
fn batch_and_single_clue_creation_produce_the_same_clue() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(HuntyCore, ());
    let creator = Address::generate(&env);

    let single_hunt = create_hunt(&env, &contract_id, &creator, "Single");
    let batch_hunt = create_hunt(&env, &contract_id, &creator, "Batch");

    env.as_contract(&contract_id, || {
        HuntyCore::add_clue(
            env.clone(),
            single_hunt,
            String::from_str(&env, "Same question"),
            String::from_str(&env, "answer"),
            10,
            true,
            Some(2),
            Some(3),
        )
        .unwrap();
        HuntyCore::add_clues(
            env.clone(),
            batch_hunt,
            vec![&env, batch_clue(&env, "Same question", 2, Some(3))],
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        let single = Storage::get_clue_or_error(&env, single_hunt, 1).unwrap();
        let batch = Storage::get_clue_or_error(&env, batch_hunt, 1).unwrap();
        assert_eq!(batch.question, single.question);
        assert_eq!(batch.points, single.points);
        assert_eq!(batch.is_required, single.is_required);
        assert_eq!(batch.difficulty, single.difficulty);
        assert_eq!(batch.weight, single.weight);
    });
}

#[test]
fn out_of_range_weights_are_rejected_for_single_and_batched_clues() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(HuntyCore, (admin,));
    let creator = Address::generate(&env);
    let hunt_id = create_hunt(&env, &contract_id, &creator, "Invalid Weight");

    for weight in [0, 11, u32::MAX] {
        env.as_contract(&contract_id, || {
            assert_eq!(
                HuntyCore::add_clue(
                    env.clone(),
                    hunt_id,
                    String::from_str(&env, "Q1"),
                    String::from_str(&env, "answer"),
                    10,
                    true,
                    Some(2),
                    Some(weight),
                ),
                Err(HuntErrorCode::InvalidWeight)
            );
        });
        env.as_contract(&contract_id, || {
            assert_eq!(
                HuntyCore::add_clues(
                    env.clone(),
                    hunt_id,
                    vec![&env, batch_clue(&env, "Q1", 2, Some(weight))],
                ),
                Err(HuntErrorCode::InvalidWeight)
            );
        });
        env.as_contract(&contract_id, || {
            // Rejected inputs must not consume clue IDs or change hunt counts.
            assert_eq!(Storage::get_clue_counter(&env, hunt_id), 0);
            let hunt = Storage::get_hunt_or_error(&env, hunt_id).unwrap();
            assert_eq!(hunt.total_clues, 0);
            assert_eq!(hunt.status, HuntStatus::Draft);
        });
    }
}

#[test]
fn single_and_batched_clues_accept_weight_boundaries_and_default() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(HuntyCore, (admin,));
    let creator = Address::generate(&env);
    let single_hunt = create_hunt(&env, &contract_id, &creator, "Single Boundaries");
    let batch_hunt = create_hunt(&env, &contract_id, &creator, "Batch Boundaries");

    for (index, weight) in [Some(1), None, Some(10)].into_iter().enumerate() {
        env.as_contract(&contract_id, || {
            HuntyCore::add_clue(
                env.clone(),
                single_hunt,
                String::from_str(&env, "Question"),
                String::from_str(&env, "answer"),
                10,
                true,
                Some(2),
                weight,
            )
            .unwrap();
        });
        env.as_contract(&contract_id, || {
            HuntyCore::add_clues(
                env.clone(),
                batch_hunt,
                vec![&env, batch_clue(&env, "Question", 2, weight)],
            )
            .unwrap();
        });
        env.as_contract(&contract_id, || {
            let clue_id = index as u32 + 1;
            let expected_weight = weight.unwrap_or(1);
            assert_eq!(
                Storage::get_clue_or_error(&env, single_hunt, clue_id)
                    .unwrap()
                    .weight,
                expected_weight
            );
            assert_eq!(
                Storage::get_clue_or_error(&env, batch_hunt, clue_id)
                    .unwrap()
                    .weight,
                expected_weight
            );
        });
    }
}

#[test]
fn maximum_weight_clues_allow_consecutive_score_updates() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(HuntyCore, (admin,));
    let creator = Address::generate(&env);
    let hunt_id = create_hunt(&env, &contract_id, &creator, "Maximum Scores");
    let player = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let mut input = batch_clue(&env, "Maximum", 5, Some(10));
        input.points = 10_000;
        HuntyCore::add_clues(env.clone(), hunt_id, vec![&env, input.clone(), input]).unwrap();
        let mut hunt = Storage::get_hunt_or_error(&env, hunt_id).unwrap();
        hunt.start_multiplier_bps = 50_000;
        let mut progress = PlayerProgress::new(&env, player.clone(), hunt_id, 0);

        for clue_id in 1..=2 {
            let clue = Storage::get_clue_or_error(&env, hunt_id, clue_id).unwrap();
            let score = HuntyCore::calculate_score(&hunt, &clue, 0, 0);
            assert_eq!(score, 2_500_000);
            progress.complete_clue(&env, clue_id, score).unwrap();
        }
        assert_eq!(progress.total_score, 5_000_000);
        assert_eq!(progress.completed_clues.len(), 2);
    });
}

#[test]
fn weighted_batched_clue_changes_the_clue_but_not_the_hunt_difficulty() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(HuntyCore, ());
    let creator = Address::generate(&env);
    let hunt_id = create_hunt(&env, &contract_id, &creator, "Weight vs Difficulty");

    env.as_contract(&contract_id, || {
        HuntyCore::add_clues(
            env.clone(),
            hunt_id,
            vec![&env, batch_clue(&env, "Q1", 2, Some(5))],
        )
        .unwrap();

        let clue = Storage::get_clue_or_error(&env, hunt_id, 1).unwrap();
        assert_eq!(clue.weight, 5);
        // Weight is a scoring multiplier, not a difficulty tier.
        assert_eq!(clue.difficulty, 2);
        assert_eq!(
            Storage::get_hunt_or_error(&env, hunt_id)
                .unwrap()
                .difficulty_rating,
            2
        );
    });
}
