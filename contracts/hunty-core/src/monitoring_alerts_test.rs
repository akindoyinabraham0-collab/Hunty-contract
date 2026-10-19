use crate::{errors::HuntErrorCode, HuntyCore, HuntyCoreClient};
use hunty_common::monitoring::Monitoring;
use soroban_sdk::{testutils::Address as _, Address, Env};

#[test]
fn only_authenticated_admin_can_clear_alerts() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let cid = env.register(HuntyCore, (admin.clone(),));
    let client = HuntyCoreClient::new(&env, &cid);
    env.as_contract(&cid, || {
        Monitoring::raise_alert(&env, "large_withdrawal")
    });

    // Supplying the real admin address without its authorization must fail.
    assert!(client.try_clear_alerts(&admin).is_err());
    assert_eq!(client.get_health_dashboard().active_alerts, 1);

    env.mock_all_auths();
    assert_eq!(
        client.try_clear_alerts(&attacker),
        Err(Ok(HuntErrorCode::Unauthorized))
    );
    assert_eq!(client.get_health_dashboard().active_alerts, 1);
    client.clear_alerts(&admin);
    assert_eq!(client.get_health_dashboard().active_alerts, 0);
    assert!(client.get_active_alerts().is_empty());
}
