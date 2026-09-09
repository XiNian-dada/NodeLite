//! Failures must converge without a new policy or connection, including partial kernel changes.

use super::*;

struct FakeTc {
    ingress: Option<u64>,
    egress: Option<u64>,
    fail_direction: Option<usize>,
    attempts: usize,
}

impl FakeTc {
    fn apply(&mut self, rate: Option<u64>) -> Result<(), TrafficControlError> {
        self.attempts += 1;
        for (direction, rule) in [&mut self.ingress, &mut self.egress]
            .into_iter()
            .enumerate()
        {
            if self.fail_direction == Some(direction) {
                self.fail_direction = None;
                return Err(TrafficControlError::Injected);
            }
            *rule = rate;
        }
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn failed_apply_and_removal_retry_without_changing_the_desired_policy() {
    for rate in [Some(2000), None] {
        for failed_direction in [0, 1] {
            let mut tc = FakeTc {
                ingress: Some(1000),
                egress: Some(1000),
                fail_direction: Some(failed_direction),
                attempts: 0,
            };
            let mut controller = TrafficController {
                last_applied_rate_kbps: Some(Some(1000)),
                ..TrafficController::default()
            };
            assert!(
                controller
                    .apply_with(rate, None, || std::future::ready(tc.apply(rate)))
                    .await
                    .is_err()
            );
            assert_eq!(
                controller.status().expect("status").state,
                TrafficControlState::Retrying
            );
            assert_eq!(
                controller.last_applied_rate_kbps, None,
                "partial changes invalidate the confirmed policy"
            );
            let (deadline, desired) = controller.retry_policy().expect("retry queued");
            assert_eq!(desired, rate);
            assert_eq!(deadline - Instant::now(), Duration::from_secs(2));
            tokio::time::advance(Duration::from_secs(2)).await;
            controller
                .apply_with(desired, None, || std::future::ready(tc.apply(desired)))
                .await
                .expect("second attempt succeeds");
            assert_eq!((tc.ingress, tc.egress), (rate, rate));
            assert_eq!(tc.attempts, 2);
            assert!(controller.retry_policy().is_none());
            assert_eq!(
                controller.status().expect("status").state,
                TrafficControlState::Applied
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn retry_delay_and_attempt_count_are_bounded_and_a_new_policy_resets_them() {
    let mut controller = TrafficController::default();
    for attempt in 1..=MAX_APPLY_ATTEMPTS {
        assert!(
            controller
                .apply_with(Some(1000), None, || async {
                    Err(TrafficControlError::Injected)
                })
                .await
                .is_err()
        );
        if attempt < MAX_APPLY_ATTEMPTS {
            let (deadline, _) = controller.retry_policy().expect("retry");
            let delay = deadline - Instant::now();
            assert!(delay >= Duration::from_secs(2) && delay <= Duration::from_secs(60));
            tokio::time::advance(delay).await;
        }
    }
    assert!(controller.retry_policy().is_none());
    assert_eq!(
        controller.status().expect("terminal status").state,
        TrafficControlState::Failed
    );
    assert!(
        controller
            .apply_with(Some(2000), None, || async {
                Err(TrafficControlError::Injected)
            })
            .await
            .is_err()
    );
    assert_eq!(
        controller.retry_policy().expect("new policy retries").0 - Instant::now(),
        Duration::from_secs(2)
    );
}
