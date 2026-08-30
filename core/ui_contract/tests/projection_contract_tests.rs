use serde_json::json;
use timem_ui_contract::projections::{
    session_worker_default_display_name, ActiveTurnProjection, CoreGlobalWorkerStatus,
    CoreSessionWorkerIdentity, CoreSessionWorkerWorkspace, FinishedTurnProjection, TurnActivity,
    TurnInputAdmission, TurnProjection, TurnProjectionOutcome, TurnToken,
};

fn token() -> TurnToken {
    TurnToken {
        session_id: "session-a".to_string(),
        turn_id: "turn-7".to_string(),
        epoch: 7,
    }
}

#[test]
fn active_turn_projection_keeps_the_existing_wire_shape() {
    let projection = TurnProjection::Active(ActiveTurnProjection {
        token: token(),
        stop_requested: false,
        input_admission: TurnInputAdmission::Open,
        activity: TurnActivity::WaitingModel { round: 2 },
    });

    assert_eq!(
        serde_json::to_value(projection).unwrap(),
        json!({
            "state": "active",
            "token": {"session_id": "session-a", "turn_id": "turn-7", "epoch": 7},
            "stop_requested": false,
            "input_admission": "open",
            "activity": {"kind": "waiting_model", "round": 2}
        })
    );
}

#[test]
fn terminal_outcomes_round_trip_without_collapsing_their_semantics() {
    let outcomes = [
        TurnProjectionOutcome::Completed,
        TurnProjectionOutcome::Cancelled,
        TurnProjectionOutcome::Failed {
            code: "model_error".to_string(),
        },
        TurnProjectionOutcome::Interrupted {
            code: "round_limit".to_string(),
        },
    ];

    for outcome in outcomes {
        let projection = TurnProjection::Finished(FinishedTurnProjection {
            token: token(),
            outcome,
        });
        let encoded = serde_json::to_string(&projection).unwrap();
        let decoded: TurnProjection = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, projection);
    }
}

#[test]
fn worker_identity_defaults_and_rename_rules_remain_ui_neutral() {
    let mut identity = CoreSessionWorkerIdentity::new_scoped(
        "session-a",
        "context-2",
        "worker-3",
        3,
        Some("  ".to_string()),
        Some("worker-1".to_string()),
    );

    assert_eq!(identity.display_name, "ID3");
    assert_eq!(session_worker_default_display_name(4, None), "ID4");
    identity.rename("  Researcher  ");
    assert_eq!(identity.display_name, "Researcher");
    identity.rename("   ");
    assert_eq!(identity.display_name, "Researcher");
}

#[test]
fn worker_workspace_and_aggregate_status_keep_existing_defaults() {
    let workspace =
        CoreSessionWorkerWorkspace::new("/tmp/data", "/tmp/audit.json", "test-runtime", "local");
    assert_eq!(workspace.current_dir, None);
    assert!(workspace.env.is_empty());
    assert!(workspace.workspace_dirs.is_empty());

    assert_eq!(
        CoreGlobalWorkerStatus::new(2),
        CoreGlobalWorkerStatus::with_session_working_worker_count(2, 2)
    );
    assert_eq!(
        CoreGlobalWorkerStatus::with_session_working_worker_count(5, 2)
            .session_working_worker_count,
        2
    );
}
