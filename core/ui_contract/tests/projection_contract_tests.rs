use serde_json::json;
use timem_ui_contract::projections::{
    ActiveTurnProjection, FinishedTurnProjection, TurnActivity, TurnInputAdmission, TurnProjection,
    TurnProjectionOutcome, TurnToken,
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
