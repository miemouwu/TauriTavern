use serde::Serialize;

use super::args::{
    ensure_visible_workspace_path, object_args, parse_workspace_path, required_trimmed_string_arg,
    tool_error,
};
use super::policy::workspace_access_policy;
use crate::application::errors::ApplicationError;
use crate::domain::models::agent::profile::ResolvedAgentProfile;
use crate::domain::models::agent::{AgentChatCommitMode, AgentToolCall, AgentToolResult};
use crate::domain::repositories::workspace_repository::WorkspaceRepository;

use super::super::dispatcher::AgentToolEffect;
use super::super::structured::structured_value;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceFinalizeStructured<'a> {
    source_path: &'a str,
    message_body_path: &'a str,
    copied: bool,
    reason: Option<&'a str>,
}

/// Publish a workspace file as the chat message and request the commit in one
/// call. Copies `source_path` into the profile's message-body artifact (e.g.
/// `scratch/render.md` -> `output/main.md`) entirely in code — the source
/// content never enters the model context — then emits `ChatCommitRequested` so
/// the host commits it (same path as `workspace.commit`). The model still calls
/// `workspace.finish` afterwards to close the run.
///
/// This collapses the orchestrator's tail (`read_file` + `write_file` +
/// `commit`) into a single round and keeps the (often large) rendered message
/// out of the orchestrator's LLM context.
pub(in crate::application::services::agent_tools) async fn finalize(
    workspace_repository: &dyn WorkspaceRepository,
    run_id: &str,
    call: &AgentToolCall,
    profile: &ResolvedAgentProfile,
) -> Result<(AgentToolResult, AgentToolEffect), ApplicationError> {
    let policy = workspace_access_policy(workspace_repository, run_id).await?;
    let args = object_args(call);

    let source = match args.and_then(|args| required_trimmed_string_arg(args, "source_path")) {
        Some(source) => source,
        None => {
            return Ok((
                tool_error(
                    call,
                    "workspace.finalize_source_required",
                    "source_path is required (e.g. scratch/render.md).",
                ),
                AgentToolEffect::None,
            ));
        }
    };
    let source_path = match parse_workspace_path(call, source) {
        Ok(path) => path,
        Err(result) => return Ok((result, AgentToolEffect::None)),
    };
    if let Err(result) = ensure_visible_workspace_path(call, &policy, &source_path) {
        return Ok((result, AgentToolEffect::None));
    }

    let message_body_path =
        match parse_workspace_path(call, profile.output.message_body_path.as_str()) {
            Ok(path) => path,
            Err(result) => return Ok((result, AgentToolEffect::None)),
        };

    let reason = args
        .and_then(|args| required_trimmed_string_arg(args, "reason"))
        .map(str::to_string);

    // Copy source -> message body in code (does not enter the model context).
    let copied = source_path.as_str() != message_body_path.as_str();
    if copied {
        let file = match workspace_repository.read_text(run_id, &source_path).await {
            Ok(file) => file,
            Err(error) => {
                return Ok((
                    tool_error(
                        call,
                        "workspace.finalize_source_unreadable",
                        &format!("cannot read source_path `{}`: {error}", source_path.as_str()),
                    ),
                    AgentToolEffect::None,
                ));
            }
        };
        workspace_repository
            .write_text(run_id, &message_body_path, &file.text)
            .await?;
    }

    Ok((
        AgentToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            content: format!(
                "Published {} to {} and requested chat commit. Call workspace.finish to close the run.",
                source_path.as_str(),
                message_body_path.as_str()
            ),
            structured: structured_value(WorkspaceFinalizeStructured {
                source_path: source_path.as_str(),
                message_body_path: message_body_path.as_str(),
                copied,
                reason: reason.as_deref(),
            }),
            is_error: false,
            error_code: None,
            resource_refs: vec![message_body_path.as_str().to_string()],
        },
        AgentToolEffect::ChatCommitRequested {
            path: message_body_path,
            mode: AgentChatCommitMode::Replace,
            reason,
        },
    ))
}
