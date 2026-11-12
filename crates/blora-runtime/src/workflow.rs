// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_events::{KnownPayload, WorkflowUpdated};
use blora_storage::{WorkflowRecord, WorkflowRunRecord};
use blora_types::{BloraError, Result, RunId, SessionId, TurnId, WorkflowId, WorkflowRunId};
use chrono::Utc;
use serde_json::{Value, json};

use crate::coordinator::Runtime;

impl Runtime {
    pub(crate) fn workflow_tool(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        arguments: &Value,
    ) -> Result<String> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| BloraError::Other("workflow needs action".to_owned()))?;
        let workflow_id = arguments
            .get("workflow_id")
            .and_then(Value::as_str)
            .map(WorkflowId::parse)
            .transpose()?;
        match action {
            "create" => {
                let name = arguments
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.trim().is_empty())
                    .ok_or_else(|| BloraError::Other("workflow create needs name".to_owned()))?;
                let definition = arguments.get("definition").cloned().ok_or_else(|| {
                    BloraError::Other("workflow create needs definition".to_owned())
                })?;
                if steps(&definition).is_empty() {
                    return Err(BloraError::Other(
                        "workflow needs at least one step".to_owned(),
                    ));
                }
                let id = WorkflowId::generate();
                let now = Utc::now();
                self.store.insert_workflow(&WorkflowRecord {
                    id: id.clone(),
                    session_id: session_id.clone(),
                    name: name.to_owned(),
                    definition_json: definition.to_string(),
                    status: "ready".to_owned(),
                    created_at: now,
                    updated_at: now,
                })?;
                self.emit_workflow(session_id, run_id, turn_id, &id, "ready", 0)?;
                Ok(format!("created workflow {id}"))
            }
            "list" => {
                let workflows = self.store.list_workflows(session_id)?;
                if workflows.is_empty() {
                    return Ok("(no workflows)".to_owned());
                }
                Ok(workflows
                    .iter()
                    .map(|workflow| {
                        format!("{}  {}  {}", workflow.id, workflow.status, workflow.name)
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "status" => {
                let workflow = self.require_workflow(workflow_id)?;
                let run = self.store.latest_workflow_run(&workflow.id)?;
                let definition: Value =
                    serde_json::from_str(&workflow.definition_json).unwrap_or(Value::Null);
                let steps = steps(&definition);
                let current = run
                    .as_ref()
                    .map_or(0, |run| run.current_step.max(0) as usize);
                let step = steps.get(current).cloned().unwrap_or(Value::Null);
                Ok(format!(
                    "{}\nstatus: {}\nstep: {}/{}\ncurrent: {}",
                    workflow.name,
                    run.as_ref().map_or("ready", |run| run.status.as_str()),
                    current.min(steps.len()),
                    steps.len(),
                    step
                ))
            }
            "start" => {
                let workflow = self.require_workflow(workflow_id)?;
                let definition: Value = serde_json::from_str(&workflow.definition_json)
                    .map_err(|err| BloraError::Other(format!("workflow definition: {err}")))?;
                let steps = steps(&definition);
                if steps.is_empty() {
                    return Err(BloraError::Other("workflow has no steps".to_owned()));
                }
                let workflow_run_id = WorkflowRunId::generate();
                self.store.insert_workflow_run(&WorkflowRunRecord {
                    id: workflow_run_id.clone(),
                    workflow_id: workflow.id.clone(),
                    status: "running".to_owned(),
                    current_step: 0,
                    state_json: json!({}).to_string(),
                    started_at: Utc::now(),
                    finished_at: None,
                })?;
                self.emit_workflow(session_id, run_id, turn_id, &workflow.id, "running", 0)?;
                Ok(format!(
                    "started workflow {} (run {workflow_run_id}); first step:\n{}",
                    workflow.id, steps[0]
                ))
            }
            "advance" => {
                let workflow = self.require_workflow(workflow_id)?;
                let mut run = self
                    .store
                    .latest_workflow_run(&workflow.id)?
                    .ok_or_else(|| BloraError::Other("workflow has not started".to_owned()))?;
                if run.status != "running" {
                    return Err(BloraError::Other(format!("workflow run is {}", run.status)));
                }
                let definition: Value =
                    serde_json::from_str(&workflow.definition_json).unwrap_or(Value::Null);
                let steps = steps(&definition);
                let next = (run.current_step + 1).max(0);
                let finished = next as usize >= steps.len();
                let status = if finished { "completed" } else { "running" };
                let state = if let Some(result) = arguments.get("result") {
                    json!({"last_result": result}).to_string()
                } else {
                    run.state_json.clone()
                };
                self.store
                    .update_workflow_run(&run.id, status, next, &state, finished)?;
                run.current_step = next;
                self.emit_workflow(session_id, run_id, turn_id, &workflow.id, status, next)?;
                if finished {
                    Ok(format!("workflow {} completed", workflow.id))
                } else {
                    Ok(format!(
                        "workflow {} step {}/{}\n{}",
                        workflow.id,
                        next + 1,
                        steps.len(),
                        steps.get(next as usize).cloned().unwrap_or(Value::Null)
                    ))
                }
            }
            "cancel" => {
                let workflow = self.require_workflow(workflow_id)?;
                if let Some(run) = self.store.latest_workflow_run(&workflow.id)? {
                    self.store.update_workflow_run(
                        &run.id,
                        "cancelled",
                        run.current_step,
                        &run.state_json,
                        true,
                    )?;
                }
                self.emit_workflow(session_id, run_id, turn_id, &workflow.id, "cancelled", 0)?;
                Ok(format!("cancelled workflow {}", workflow.id))
            }
            "delete" => {
                let workflow = self.require_workflow(workflow_id)?;
                self.store.delete_workflow(&workflow.id)?;
                Ok(format!("deleted workflow {}", workflow.id))
            }
            other => Err(BloraError::Other(format!(
                "unknown workflow action: {other}"
            ))),
        }
    }

    fn require_workflow(&self, id: Option<WorkflowId>) -> Result<WorkflowRecord> {
        let id =
            id.ok_or_else(|| BloraError::Other("workflow action needs workflow_id".to_owned()))?;
        self.store
            .get_workflow(&id)?
            .ok_or_else(|| BloraError::Other("workflow not found".to_owned()))
    }

    fn emit_workflow(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        workflow_id: &WorkflowId,
        status: &str,
        step: i64,
    ) -> Result<()> {
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::WorkflowUpdated(WorkflowUpdated {
                workflow_id: workflow_id.clone(),
                status: status.to_owned(),
                step,
            }),
        )
    }
}

fn steps(definition: &Value) -> Vec<Value> {
    definition
        .get("steps")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| definition.as_array().cloned())
        .unwrap_or_default()
}
