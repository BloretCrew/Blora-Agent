// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::sync::Arc;
use std::thread;

use blora_events::{KnownPayload, NewEvent, UserInput};
use blora_runtime::Runtime;
use blora_storage::{CreateSession, InMemoryStore, RuntimeStore, SqliteStore};
use blora_types::{ApprovalId, Mode};
use chrono::Utc;

fn exercise_core_store(store: &dyn RuntimeStore) {
    let session_id = store
        .create_session(CreateSession {
            title: Some("contract".to_owned()),
            workspace_path: "/memory/workspace".to_owned(),
            mode: Mode::Code,
            parent_session_id: None,
        })
        .unwrap();
    assert_eq!(store.last_sequence(&session_id).unwrap(), 1);

    let event = store
        .append(NewEvent::new(
            session_id.clone(),
            KnownPayload::UserInput(UserInput {
                text: "Hello".to_owned(),
                ..Default::default()
            }),
        ))
        .unwrap();
    assert_eq!(event.sequence, 2);
    assert_eq!(
        store.load_events_after(&session_id, 1).unwrap()[0].sequence,
        2
    );
    assert_eq!(store.load_projection(&session_id).unwrap().last_sequence, 2);

    store.queue_steer(&session_id, "first").unwrap();
    store.queue_steer(&session_id, "second").unwrap();
    assert_eq!(store.take_steers(&session_id).unwrap(), ["first", "second"]);
    assert!(store.take_steers(&session_id).unwrap().is_empty());

    let approval = blora_storage::ApprovalRecord {
        id: ApprovalId::generate(),
        session_id: session_id.clone(),
        run_id: None,
        capability: "write_file".to_owned(),
        summary: "write file".to_owned(),
        status: "pending".to_owned(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store.insert_approval(&approval).unwrap();
    assert_eq!(store.list_pending_approvals(&session_id).unwrap().len(), 1);

    let wins = thread::scope(|scope| {
        let first = scope.spawn(|| {
            store
                .resolve_approval_row(&approval.id, "allow", Utc::now())
                .unwrap()
        });
        let second = scope.spawn(|| {
            store
                .resolve_approval_row(&approval.id, "allow", Utc::now())
                .unwrap()
        });
        [first.join().unwrap(), second.join().unwrap()]
            .into_iter()
            .filter(|won| *won)
            .count()
    });
    assert_eq!(wins, 1, "approval is a compare-and-set transition");
    assert!(
        store
            .list_pending_approvals(&session_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn sqlite_and_memory_stores_share_the_core_contract() {
    let sqlite = SqliteStore::open_in_memory().unwrap();
    exercise_core_store(&sqlite);
    let memory = InMemoryStore::new().unwrap();
    exercise_core_store(&memory);
}

#[test]
fn runtime_accepts_an_in_memory_store_trait_object() {
    let store = Arc::new(InMemoryStore::new().unwrap());
    let runtime = Runtime::from_store(store, blora_runtime::RuntimeConfig::sdk_default());
    let session = runtime
        .create_session(CreateSession {
            title: Some("runtime".to_owned()),
            workspace_path: "/memory/runtime".to_owned(),
            mode: Mode::Code,
            parent_session_id: None,
        })
        .unwrap();
    assert_eq!(runtime.last_sequence(&session).unwrap(), 1);
}
