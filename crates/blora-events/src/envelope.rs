// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::collections::BTreeMap;

use blora_types::{
    Actor, Clock, EventId, Result, RunId, SCHEMA_VERSION, SessionId, SystemClock, TurnId,
    Visibility,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::payload::KnownPayload;

/// Event to persist. Storage assigns `event_id`, `sequence`, and timestamp.
#[derive(Clone, Debug)]
pub struct NewEvent {
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub turn_id: Option<TurnId>,
    pub parent_event_id: Option<EventId>,
    pub causation_id: Option<EventId>,
    pub actor: Actor,
    pub visibility: Visibility,
    pub payload: KnownPayload,
    pub metadata: BTreeMap<String, Value>,
}

impl NewEvent {
    #[must_use]
    pub fn new(session_id: SessionId, payload: KnownPayload) -> Self {
        let actor = payload.default_actor();
        Self {
            session_id,
            run_id: None,
            turn_id: None,
            parent_event_id: None,
            causation_id: None,
            actor,
            visibility: Visibility::User,
            payload,
            metadata: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_run(mut self, run_id: RunId) -> Self {
        self.run_id = Some(run_id);
        self
    }

    #[must_use]
    pub fn with_turn(mut self, turn_id: TurnId) -> Self {
        self.turn_id = Some(turn_id);
        self
    }

    #[must_use]
    pub fn with_visibility(mut self, visibility: Visibility) -> Self {
        self.visibility = visibility;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: EventId,
    pub schema_version: u32,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub turn_id: Option<TurnId>,
    pub parent_event_id: Option<EventId>,
    pub causation_id: Option<EventId>,
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub actor: Actor,
    pub visibility: Visibility,
    #[serde(rename = "type")]
    pub event_type: String,
    pub payload: Value,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl EventEnvelope {
    pub fn from_new(new: NewEvent, sequence: u64, clock: &impl Clock) -> Result<Self> {
        Self::from_new_with_id(new, EventId::generate(), sequence, clock)
    }

    pub fn from_new_with_id(
        new: NewEvent,
        event_id: EventId,
        sequence: u64,
        clock: &impl Clock,
    ) -> Result<Self> {
        if sequence == 0 {
            return Err(blora_types::BloraError::event(
                "event sequence must start at 1",
            ));
        }
        Ok(Self {
            event_id,
            schema_version: SCHEMA_VERSION,
            session_id: new.session_id,
            run_id: new.run_id,
            turn_id: new.turn_id,
            parent_event_id: new.parent_event_id,
            causation_id: new.causation_id,
            sequence,
            timestamp: clock.now(),
            actor: new.actor,
            visibility: new.visibility,
            event_type: new.payload.event_type().to_owned(),
            payload: new.payload.to_value()?,
            metadata: new.metadata,
        })
    }

    pub fn decode_payload(&self) -> Result<Option<KnownPayload>> {
        KnownPayload::from_type(&self.event_type, &self.payload)
    }

    pub fn validate(&self) -> Result<()> {
        EventId::parse(self.event_id.as_str())?;
        SessionId::parse(self.session_id.as_str())?;
        if self.sequence == 0 {
            return Err(blora_types::BloraError::event(
                "event sequence must start at 1",
            ));
        }
        if self.schema_version == 0 {
            return Err(blora_types::BloraError::event(
                "schema_version must be >= 1",
            ));
        }
        if self.event_type.is_empty() {
            return Err(blora_types::BloraError::event(
                "event type must not be empty",
            ));
        }
        Ok(())
    }
}

impl EventEnvelope {
    pub fn stamped(new: NewEvent, sequence: u64) -> Result<Self> {
        Self::from_new(new, sequence, &SystemClock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UserInput;
    use serde_json::json;

    #[test]
    fn unknown_event_types_are_skipped_not_fatal() {
        let envelope = EventEnvelope {
            event_id: EventId::generate(),
            schema_version: 1,
            session_id: SessionId::generate(),
            run_id: None,
            turn_id: None,
            parent_event_id: None,
            causation_id: None,
            sequence: 2,
            timestamp: Utc::now(),
            actor: Actor::System,
            visibility: Visibility::Internal,
            event_type: "future.experimental".to_owned(),
            payload: json!({"ok": true}),
            metadata: BTreeMap::new(),
        };
        assert!(envelope.decode_payload().unwrap().is_none());
    }

    #[test]
    fn known_payload_roundtrip() {
        let session_id = SessionId::generate();
        let envelope = EventEnvelope::stamped(
            NewEvent::new(
                session_id,
                KnownPayload::UserInput(UserInput {
                    text: "hello".to_owned(),
                }),
            ),
            1,
        )
        .unwrap();
        let decoded = envelope.decode_payload().unwrap().unwrap();
        assert_eq!(decoded.event_type(), "user.input");
    }

    #[test]
    fn routing_changed_roundtrip() {
        let session_id = SessionId::generate();
        let envelope = EventEnvelope::stamped(
            NewEvent::new(
                session_id,
                KnownPayload::RoutingChanged(crate::RoutingChanged {
                    from_provider: Some("blora".to_owned()),
                    to_provider: "crewrouter".to_owned(),
                    from_model: Some("blora".to_owned()),
                    to_model: "fusion".to_owned(),
                }),
            ),
            1,
        )
        .unwrap();
        let decoded = envelope.decode_payload().unwrap().unwrap();
        assert_eq!(decoded.event_type(), "routing.changed");
    }
}
