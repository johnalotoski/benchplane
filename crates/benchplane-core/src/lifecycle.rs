// SPDX-License-Identifier: Apache-2.0

use benchplane_schema::{FailureRecord, LifecycleEvent, RunState};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LifecycleError {
    #[error("lifecycle.invalidTransition: invalid transition from {from:?} to {to:?}")]
    InvalidTransition { from: RunState, to: RunState },
}

#[derive(Debug, Clone)]
pub struct Lifecycle {
    run_id: String,
    state: RunState,
    next_sequence: u64,
    attempt_number: u32,
}

impl Lifecycle {
    pub fn new(run_id: String, recorded_at: String, attempt_number: u32) -> (Self, LifecycleEvent) {
        let lifecycle = Self {
            run_id: run_id.clone(),
            state: RunState::Created,
            next_sequence: 2,
            attempt_number,
        };
        let event = LifecycleEvent {
            run_id,
            sequence: 1,
            recorded_at,
            from_state: None,
            to_state: RunState::Created,
            attempt_number,
            failure: None,
        };
        (lifecycle, event)
    }

    pub fn state(&self) -> RunState {
        self.state
    }

    pub fn transition(
        &mut self,
        to: RunState,
        recorded_at: String,
        failure: Option<FailureRecord>,
    ) -> Result<LifecycleEvent, LifecycleError> {
        if !is_valid_transition(self.state, to) {
            return Err(LifecycleError::InvalidTransition {
                from: self.state,
                to,
            });
        }

        let event = LifecycleEvent {
            run_id: self.run_id.clone(),
            sequence: self.next_sequence,
            recorded_at,
            from_state: Some(self.state),
            to_state: to,
            attempt_number: self.attempt_number,
            failure,
        };
        self.state = to;
        self.next_sequence += 1;
        Ok(event)
    }
}

fn is_valid_transition(from: RunState, to: RunState) -> bool {
    matches!(
        (from, to),
        (RunState::Created, RunState::Preparing)
            | (RunState::Preparing, RunState::Running)
            | (RunState::Preparing, RunState::Finalizing)
            | (RunState::Running, RunState::Collecting)
            | (RunState::Running, RunState::Finalizing)
            | (RunState::Collecting, RunState::Finalizing)
            | (RunState::Finalizing, RunState::Succeeded)
            | (RunState::Finalizing, RunState::Failed)
            | (RunState::Finalizing, RunState::Interrupted)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_documented_transition_graph() {
        let (mut lifecycle, initial) = Lifecycle::new(
            "run-018f6f9a-7b3c-7abc-8def-0123456789ab".into(),
            "2026-01-01T00:00:00.000Z".into(),
            1,
        );
        assert_eq!(initial.from_state, None);
        assert_eq!(initial.to_state, RunState::Created);

        for state in [
            RunState::Preparing,
            RunState::Running,
            RunState::Collecting,
            RunState::Finalizing,
            RunState::Succeeded,
        ] {
            lifecycle
                .transition(state, "2026-01-01T00:00:00.001Z".into(), None)
                .expect("transition should succeed");
        }
        assert_eq!(lifecycle.state(), RunState::Succeeded);
    }

    #[test]
    fn rejected_transition_does_not_change_state_or_sequence() {
        let (mut lifecycle, _) = Lifecycle::new(
            "run-018f6f9a-7b3c-7abc-8def-0123456789ab".into(),
            "2026-01-01T00:00:00.000Z".into(),
            1,
        );
        let error = lifecycle
            .transition(RunState::Succeeded, "2026-01-01T00:00:00.001Z".into(), None)
            .expect_err("created to succeeded must be rejected");
        assert_eq!(
            error,
            LifecycleError::InvalidTransition {
                from: RunState::Created,
                to: RunState::Succeeded,
            }
        );
        assert_eq!(lifecycle.state(), RunState::Created);

        let event = lifecycle
            .transition(RunState::Preparing, "2026-01-01T00:00:00.002Z".into(), None)
            .expect("valid transition should succeed");
        assert_eq!(event.sequence, 2);
    }
}
