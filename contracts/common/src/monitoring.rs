#![allow(deprecated)]

use soroban_sdk::{contracttype, symbol_short, Address, Env, Map, String, Vec};

const INVOCATIONS_KEY: soroban_sdk::Symbol = symbol_short!("INVCT");
const FAILURES_KEY: soroban_sdk::Symbol = symbol_short!("FAILCT");
const GAS_UNITS_KEY: soroban_sdk::Symbol = symbol_short!("GASUN");
const ALERTS_KEY: soroban_sdk::Symbol = symbol_short!("ALERT");
const LARGE_WD_THRESHOLD_MAP: soroban_sdk::Symbol = symbol_short!("LGWDMAP");
/// Per-kind alert counters: `Map<alert_type, HealthAlert>`.
const ALERT_MAP_KEY: soroban_sdk::Symbol = symbol_short!("ALERTMAP");
const ALERT_TTL_THRESHOLD: u32 = 172_800;
const ALERT_TTL_EXTEND_TO: u32 = 518_400;

/// Default large-withdrawal alert threshold, calibrated for the native XLM
/// token: 100 XLM (1_000_000_000 stroops; 1 XLM = 10_000_000 stroops).
/// Other tokens MUST configure their own threshold via
/// `set_large_withdrawal_threshold`: amounts are raw token units, so a
/// stroop-denominated constant is meaningless for non-XLM assets.
const LARGE_WITHDRAWAL_THRESHOLD: i128 = 1_000_000_000;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractHealth {
    pub total_invocations: u64,
    pub failed_invocations: u64,
    pub failure_rate_bps: u32,
    pub avg_gas_units: u64,
    /// Alert occurrences since the last administrative clear.
    pub active_alerts: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthAlert {
    pub alert_type: String,
    pub count: u32,
    pub last_ledger: u64,
}

pub struct Monitoring;

impl Monitoring {
    /// Read legacy instance counters until the next mutation migrates them.
    fn alert_map(env: &Env) -> Map<String, HealthAlert> {
        if let Some(alerts) = env.storage().persistent().get(&ALERT_MAP_KEY) {
            // Regular monitoring must keep unacknowledged alerts live even
            // when no new alerts are raised during the retention period.
            Self::extend_alert_ttl(env);
            alerts
        } else {
            env.storage()
                .instance()
                .get(&ALERT_MAP_KEY)
                .unwrap_or_else(|| Map::new(env))
        }
    }

    fn extend_alert_ttl(env: &Env) {
        let extend_to = ALERT_TTL_EXTEND_TO.min(env.storage().max_ttl());
        let threshold = ALERT_TTL_THRESHOLD.min(extend_to);
        env.storage()
            .persistent()
            .extend_ttl(&ALERT_MAP_KEY, threshold, extend_to);
    }

    fn save_alert_map(env: &Env, alerts: &Map<String, HealthAlert>) {
        env.storage().persistent().set(&ALERT_MAP_KEY, alerts);
        Self::extend_alert_ttl(env);
        env.storage().instance().remove(&ALERT_MAP_KEY);
        env.storage().instance().remove(&ALERTS_KEY);
    }

    /// Acknowledge all current alerts. Callers must enforce their contract's
    /// admin authorization before calling this shared helper.
    pub fn clear_alerts(env: &Env) {
        let count = Self::health_dashboard(env).active_alerts;
        env.storage().persistent().remove(&ALERT_MAP_KEY);
        env.storage().instance().remove(&ALERT_MAP_KEY);
        env.storage().instance().remove(&ALERTS_KEY);
        env.events().publish(
            (symbol_short!("alert"), symbol_short!("cleared")),
            (count, env.ledger().sequence()),
        );
    }

    /// Telemetry record. Emits an event for off-chain indexers AND updates the
    /// on-chain counters (`INVCT`/`FAILCT`/`GASUN`) that `health_dashboard`
    /// reads, so the dashboard reflects every recorded invocation.
    /// `gas_units` is a caller-supplied estimate: contracts cannot observe
    /// their own consumed budget on-chain, so callers pass the measured
    /// off-chain cost for their entrypoint.
    pub fn record_invocation_event(env: &Env, gas_units: u64, succeeded: bool) {
        let total: u64 = env.storage().instance().get(&INVOCATIONS_KEY).unwrap_or(0);
        env.storage().instance().set(&INVOCATIONS_KEY, &(total + 1));

        let gas_total: u64 = env.storage().instance().get(&GAS_UNITS_KEY).unwrap_or(0);
        env.storage()
            .instance()
            .set(&GAS_UNITS_KEY, &(gas_total.saturating_add(gas_units)));

        if !succeeded {
            let failures: u64 = env.storage().instance().get(&FAILURES_KEY).unwrap_or(0);
            env.storage().instance().set(&FAILURES_KEY, &(failures + 1));
        }

        env.events().publish(
            (symbol_short!("invoke"), symbol_short!("telemetry")),
            (gas_units, succeeded),
        );

        if !succeeded {
            Self::raise_alert(env, "invocation_failure");
        }
    }

    /// On-chain counter variant. Kept for backwards compatibility; delegates
    /// to `record_invocation_event` so both paths share one code path.
    pub fn record_invocation(env: &Env, gas_units: u64, succeeded: bool) {
        Self::record_invocation_event(env, gas_units, succeeded);
    }

    /// Native-token threshold check. Only meaningful for XLM-denominated
    /// amounts; prefer `record_large_withdrawal_for_token` with an explicit
    /// token so per-token thresholds apply.
    pub fn record_large_withdrawal(env: &Env, amount: i128) {
        if amount > LARGE_WITHDRAWAL_THRESHOLD {
            Self::raise_alert(env, "large_withdrawal");
        }
    }

    /// Admin-configurable per-token threshold (raw token units). Callers are
    /// responsible for gating this behind their own admin auth.
    pub fn set_large_withdrawal_threshold(env: &Env, token: &Address, threshold: i128) {
        let mut thresholds: Map<Address, i128> = env
            .storage()
            .instance()
            .get(&LARGE_WD_THRESHOLD_MAP)
            .unwrap_or_else(|| Map::new(env));
        thresholds.set(token.clone(), threshold);
        env.storage()
            .instance()
            .set(&LARGE_WD_THRESHOLD_MAP, &thresholds);
    }

    /// Effective threshold for a token: configured value, else the XLM default.
    pub fn get_large_withdrawal_threshold(env: &Env, token: &Address) -> i128 {
        env.storage()
            .instance()
            .get::<_, Map<Address, i128>>(&LARGE_WD_THRESHOLD_MAP)
            .and_then(|m| m.get(token.clone()))
            .unwrap_or(LARGE_WITHDRAWAL_THRESHOLD)
    }

    /// Token-aware large-withdrawal check using the per-token threshold.
    pub fn record_large_withdrawal_for_token(env: &Env, token: &Address, amount: i128) {
        if amount > Self::get_large_withdrawal_threshold(env, token) {
            Self::raise_alert(env, "large_withdrawal");
        }
    }

    /// Records an alert *per kind* so an operator can tell seven failed
    /// invocations apart from seven suspicious withdrawals, and emits an event
    /// so off-chain monitoring can react without polling.
    pub fn raise_alert(env: &Env, kind: &str) {
        let alert_type = String::from_str(env, kind);
        let ledger = env.ledger().sequence() as u64;

        let mut alerts = Self::alert_map(env);

        let count = alerts
            .get(alert_type.clone())
            .map(|a: HealthAlert| a.count)
            .unwrap_or(0)
            .saturating_add(1);

        alerts.set(
            alert_type.clone(),
            HealthAlert {
                alert_type: alert_type.clone(),
                count,
                last_ledger: ledger,
            },
        );
        Self::save_alert_map(env, &alerts);

        env.events().publish(
            (symbol_short!("alert"), symbol_short!("raised")),
            (alert_type, count, ledger),
        );
    }

    /// Alert kinds raised since the last clear, with their occurrence counts.
    pub fn active_alerts(env: &Env) -> Vec<HealthAlert> {
        let alerts = Self::alert_map(env);

        let mut out: Vec<HealthAlert> = Vec::new(env);
        for (_, alert) in alerts.iter() {
            out.push_back(alert);
        }
        out
    }

    pub fn health_dashboard(env: &Env) -> ContractHealth {
        let total: u64 = env.storage().instance().get(&INVOCATIONS_KEY).unwrap_or(0);
        let failures: u64 = env.storage().instance().get(&FAILURES_KEY).unwrap_or(0);
        let gas_total: u64 = env.storage().instance().get(&GAS_UNITS_KEY).unwrap_or(0);
        let mut alerts = 0u32;
        for (_, alert) in Self::alert_map(env).iter() {
            alerts = alerts.saturating_add(alert.count);
        }

        let failure_rate_bps = if let Some(rate) = failures
            .checked_mul(10_000)
            .and_then(|n| n.checked_div(total))
        {
            rate as u32
        } else {
            0
        };
        let avg_gas_units = gas_total.checked_div(total).unwrap_or(0);

        ContractHealth {
            total_invocations: total,
            failed_invocations: failures,
            failure_rate_bps,
            avg_gas_units,
            active_alerts: alerts,
        }
    }
}
