use crate::monitoring::{HealthAlert, Monitoring};
use soroban_sdk::{
    contract, contractimpl, symbol_short,
    testutils::{Events as _, Ledger as _},
    xdr, Env, Map, String, Symbol, TryFromVal, Val,
};

#[contract]
struct MonitoredContract;

#[contractimpl]
impl MonitoredContract {
    pub fn ping() {}
}

fn assert_last_clear_event(env: &Env, count: u32, ledger: u32) {
    let events = env.events().all();
    let event = events.events().last().unwrap();
    let xdr::ContractEventBody::V0(body) = &event.body else {
        panic!("unexpected event body");
    };
    assert_eq!(body.topics.len(), 2);
    let first = Val::try_from_val(env, &body.topics[0]).unwrap();
    let second = Val::try_from_val(env, &body.topics[1]).unwrap();
    assert_eq!(
        Symbol::try_from_val(env, &first).unwrap(),
        symbol_short!("alert")
    );
    assert_eq!(
        Symbol::try_from_val(env, &second).unwrap(),
        symbol_short!("cleared")
    );
    let data = Val::try_from_val(env, &body.data).unwrap();
    assert_eq!(
        <(u32, u32)>::try_from_val(env, &data).unwrap(),
        (count, ledger)
    );
}

#[test]
fn clear_resets_all_alert_kinds_and_preserves_invocation_metrics() {
    let env = Env::default();
    let cid = env.register(MonitoredContract, ());
    env.ledger().set_sequence_number(100);
    env.as_contract(&cid, || {
        Monitoring::record_invocation(&env, 100, false);
        Monitoring::record_invocation(&env, 300, true);
        Monitoring::raise_alert(&env, "large_withdrawal");
        Monitoring::raise_alert(&env, "large_withdrawal");
        let before = Monitoring::health_dashboard(&env);
        assert_eq!(before.active_alerts, 3);
        assert_eq!(Monitoring::active_alerts(&env).len(), 2);

        Monitoring::clear_alerts(&env);
        assert_last_clear_event(&env, 3, 100);
        assert!(Monitoring::active_alerts(&env).is_empty());
        let after = Monitoring::health_dashboard(&env);
        assert_eq!(after.active_alerts, 0);
        assert_eq!(after.total_invocations, before.total_invocations);
        assert_eq!(after.failed_invocations, before.failed_invocations);
        assert_eq!(after.failure_rate_bps, before.failure_rate_bps);
        assert_eq!(after.avg_gas_units, before.avg_gas_units);

        Monitoring::clear_alerts(&env);
        assert_last_clear_event(&env, 0, 100);
        Monitoring::raise_alert(&env, "large_withdrawal");
        let alert = Monitoring::active_alerts(&env).get(0).unwrap();
        assert_eq!(alert.count, 1);
        assert_eq!(alert.last_ledger, 100);
        assert_eq!(Monitoring::health_dashboard(&env).active_alerts, 1);
    });
}

#[test]
fn legacy_alerts_are_read_and_migrated_on_next_raise() {
    let env = Env::default();
    let cid = env.register(MonitoredContract, ());
    env.as_contract(&cid, || {
        let key = symbol_short!("ALERTMAP");
        let kind = String::from_str(&env, "large_withdrawal");
        let mut alerts = Map::new(&env);
        alerts.set(
            kind.clone(),
            HealthAlert {
                alert_type: kind.clone(),
                count: 4,
                last_ledger: 1,
            },
        );
        env.storage().instance().set(&key, &alerts);
        env.storage().instance().set(&symbol_short!("ALERT"), &4u32);
        assert_eq!(Monitoring::health_dashboard(&env).active_alerts, 4);
        assert_eq!(Monitoring::active_alerts(&env).get(0).unwrap().count, 4);
        assert!(!env.storage().persistent().has(&key));

        Monitoring::raise_alert(&env, "large_withdrawal");
        let migrated: Map<String, HealthAlert> = env.storage().persistent().get(&key).unwrap();
        assert_eq!(migrated.get(kind).unwrap().count, 5);
        assert!(!env.storage().instance().has(&key));
        assert!(!env.storage().instance().has(&symbol_short!("ALERT")));
        assert_eq!(Monitoring::health_dashboard(&env).active_alerts, 5);
    });
}

#[test]
fn clear_removes_legacy_alerts_without_requiring_another_raise() {
    let env = Env::default();
    let cid = env.register(MonitoredContract, ());
    env.as_contract(&cid, || {
        let key = symbol_short!("ALERTMAP");
        let kind = String::from_str(&env, "invocation_failure");
        let mut alerts = Map::new(&env);
        alerts.set(
            kind.clone(),
            HealthAlert {
                alert_type: kind,
                count: 2,
                last_ledger: 1,
            },
        );
        env.storage().instance().set(&key, &alerts);
        env.storage().instance().set(&symbol_short!("ALERT"), &2u32);
        Monitoring::clear_alerts(&env);
        assert_last_clear_event(&env, 2, env.ledger().sequence());
        assert!(!env.storage().instance().has(&key));
        assert!(!env.storage().instance().has(&symbol_short!("ALERT")));
        assert!(!env.storage().persistent().has(&key));
        assert_eq!(Monitoring::health_dashboard(&env).active_alerts, 0);
        assert!(Monitoring::active_alerts(&env).is_empty());
    });
}
