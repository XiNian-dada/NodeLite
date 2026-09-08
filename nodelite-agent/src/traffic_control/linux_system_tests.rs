//! Run only inside scripts/test-traffic-control-systemd.sh's disposable network namespace.

use super::*;

#[tokio::test]
#[ignore = "requires the isolated systemd/network namespace harness"]
async fn applies_changes_and_removes_only_owned_filters() {
    assert_eq!(std::env::var("NODELITE_TC_SYSTEM_TEST").as_deref(), Ok("1"));
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "the real unit must run unprivileged"
    );
    let mut controller = TrafficController::default();
    for rate in [Some(1000), Some(2000), None] {
        assert_eq!(
            controller.apply(rate).await.expect("apply policy"),
            TrafficControlOutcome::Applied
        );
        for direction in ["ingress", "egress"] {
            let filters = tc_output(
                "inspect test filters",
                filter_show_args("nltest0", direction),
            )
            .expect("read filters");
            assert!(
                filters.contains("pref 42"),
                "foreign filter was removed: {filters}"
            );
            assert_eq!(has_nodelite_police_filter(&filters), rate.is_some());
            if let Some(rate) = rate {
                assert!(
                    filters.contains(&format!("rate {}Mbit", rate / 1000)),
                    "rate was not changed: {filters}"
                );
            }
        }
    }
}

#[test]
#[ignore = "requires the isolated systemd/network namespace harness"]
fn disabled_unit_has_no_network_admin_capability() {
    assert_eq!(std::env::var("NODELITE_TC_SYSTEM_TEST").as_deref(), Ok("1"));
    assert_ne!(unsafe { libc::geteuid() }, 0);
    assert!(!capability::has_net_admin(
        &std::fs::read_to_string("/proc/self/status").expect("capabilities")
    ));
    assert_eq!(
        capability::unavailable_reason(),
        Some(nodelite_proto::TrafficControlUnavailableReason::Disabled)
    );
}
