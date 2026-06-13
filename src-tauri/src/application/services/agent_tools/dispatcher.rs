use std::sync::Arc;
use std::time::Instant;

use super::chat;
use super::dice;
use super::memory;
use super::session::AgentToolSession;
use super::skill;
use super::structured::{ToolErrorStructured, structured_value};
use super::workspace;
use super::world_info;
use crate::application::errors::ApplicationError;
use crate::application::services::skill_service::SkillService;
use crate::domain::models::agent::profile::ResolvedAgentProfile;
use crate::domain::models::agent::{
    AgentChatCommitMode, AgentToolCall, AgentToolResult, WorkspaceFileWriteMode, WorkspacePath,
};
use crate::domain::repositories::agent_run_repository::AgentRunRepository;
use crate::domain::repositories::chat_repository::ChatRepository;
use crate::domain::repositories::group_chat_repository::GroupChatRepository;
use crate::domain::repositories::workspace_repository::{WorkspaceFile, WorkspaceRepository};
use crate::infrastructure::memory::{MemoryStore, MemoryStoreProvider};

const RUN_PROMPT_SNAPSHOT_PATH: &str = "input/prompt_snapshot.json";

#[derive(Debug, Clone)]
pub struct AgentToolDispatchOutcome {
    pub result: AgentToolResult,
    pub effect: AgentToolEffect,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone)]
pub enum AgentToolEffect {
    None,
    WorkspaceFileWritten {
        file: WorkspaceFile,
        mode: WorkspaceFileWriteMode,
    },
    WorkspaceFilePatched {
        file: WorkspaceFile,
        replacements: usize,
        old_sha256: String,
    },
    ChatCommitRequested {
        path: WorkspacePath,
        mode: AgentChatCommitMode,
        reason: Option<String>,
    },
    ChatCommitted {
        path: WorkspacePath,
        mode: AgentChatCommitMode,
        message_id: Option<String>,
    },
    TaskReturned {
        status: crate::domain::models::agent::AgentTaskStatus,
        result_ref: WorkspacePath,
        summary: String,
    },
    Finish,
}

pub struct AgentToolDispatcher {
    run_repository: Arc<dyn AgentRunRepository>,
    chat_repository: Arc<dyn ChatRepository>,
    group_chat_repository: Arc<dyn GroupChatRepository>,
    workspace_repository: Arc<dyn WorkspaceRepository>,
    skill_service: Arc<SkillService>,
    memory_provider: Arc<MemoryStoreProvider>,
}

impl AgentToolDispatcher {
    pub fn new(
        run_repository: Arc<dyn AgentRunRepository>,
        chat_repository: Arc<dyn ChatRepository>,
        group_chat_repository: Arc<dyn GroupChatRepository>,
        workspace_repository: Arc<dyn WorkspaceRepository>,
        skill_service: Arc<SkillService>,
        memory_provider: Arc<MemoryStoreProvider>,
    ) -> Self {
        Self {
            run_repository,
            chat_repository,
            group_chat_repository,
            workspace_repository,
            skill_service,
            memory_provider,
        }
    }

    /// Resolve the per-chat [`MemoryStore`] for a run by loading the run and
    /// opening (or reusing) the store keyed on its `stable_chat_id`. Mirrors the
    /// run->chat resolution used by the `chat.*` tools; the store lives outside
    /// the per-run workspace copy so the model cannot write to it directly.
    async fn memory_store_for(
        &self,
        run_id: &str,
    ) -> Result<std::sync::Arc<MemoryStore>, ApplicationError> {
        let run = self.run_repository.load_run(run_id).await?;
        self.memory_provider
            .get_or_open(&run.stable_chat_id)
            .await
            .map_err(|e| ApplicationError::InternalError(format!("memory store open failed: {e:?}")))
    }

    pub async fn dispatch(
        &self,
        run_id: &str,
        call: &AgentToolCall,
        session: &mut AgentToolSession,
        profile: &ResolvedAgentProfile,
    ) -> Result<AgentToolDispatchOutcome, ApplicationError> {
        self.dispatch_with_model_workspace_repository(
            run_id,
            call,
            session,
            profile,
            self.workspace_repository.as_ref(),
        )
        .await
    }

    pub(crate) async fn dispatch_with_model_workspace_repository(
        &self,
        run_id: &str,
        call: &AgentToolCall,
        session: &mut AgentToolSession,
        profile: &ResolvedAgentProfile,
        model_workspace_repository: &dyn WorkspaceRepository,
    ) -> Result<AgentToolDispatchOutcome, ApplicationError> {
        let started = Instant::now();
        let outcome = match call.name.as_str() {
            chat::CHAT_SEARCH => {
                chat::search(
                    self.run_repository.as_ref(),
                    self.chat_repository.as_ref(),
                    self.group_chat_repository.as_ref(),
                    run_id,
                    call,
                )
                .await?
            }
            chat::CHAT_READ_MESSAGES => {
                chat::read_messages(
                    self.run_repository.as_ref(),
                    self.chat_repository.as_ref(),
                    self.group_chat_repository.as_ref(),
                    run_id,
                    call,
                )
                .await?
            }
            world_info::WORLDINFO_READ_ACTIVATED => {
                // WorldInfo activation is a hidden run input fact, not a model-visible
                // workspace file; invocation workspace policy must not gate this read.
                let prompt_snapshot = self.read_run_prompt_snapshot(run_id).await?;
                world_info::read_activated(&prompt_snapshot, call)?
            }
            dice::DICE_ROLL => dice::roll(call).await?,
            skill::SKILL_LIST => skill::list(call, session, profile).await?,
            skill::SKILL_SEARCH => {
                skill::search(self.skill_service.as_ref(), call, session, profile).await?
            }
            skill::SKILL_READ => {
                skill::read(self.skill_service.as_ref(), call, session, profile).await?
            }
            workspace::WORKSPACE_LIST_FILES => {
                workspace::list_files(model_workspace_repository, run_id, call).await?
            }
            workspace::WORKSPACE_SEARCH_FILES => {
                workspace::search_files(model_workspace_repository, run_id, call).await?
            }
            workspace::WORKSPACE_READ_FILE => {
                workspace::read_file(model_workspace_repository, run_id, call, session).await?
            }
            workspace::WORKSPACE_WRITE_FILE => {
                workspace::write_file(model_workspace_repository, run_id, call, session).await?
            }
            workspace::WORKSPACE_APPLY_PATCH => {
                workspace::apply_patch(model_workspace_repository, run_id, call, session).await?
            }
            workspace::WORKSPACE_COMMIT => {
                workspace::commit(model_workspace_repository, run_id, call, profile).await?
            }
            workspace::WORKSPACE_FINISH => workspace::finish(call)?,
            memory::MEMORY_SEARCH => {
                let store = self.memory_store_for(run_id).await?;
                memory::search(&store, call).await?
            }
            memory::MEMORY_READ => {
                let store = self.memory_store_for(run_id).await?;
                memory::read(&store, call).await?
            }
            memory::MEMORY_TIMELINE => {
                let store = self.memory_store_for(run_id).await?;
                memory::timeline(&store, call).await?
            }
            memory::MEMORY_PROPOSE => {
                let store = self.memory_store_for(run_id).await?;
                memory::propose(&store, call).await?
            }
            other => {
                let message = format!("Unknown or unavailable tool `{other}`.");
                (
                    AgentToolResult {
                        call_id: call.id.clone(),
                        name: call.name.clone(),
                        content: message.clone(),
                        structured: structured_value(ToolErrorStructured::new(
                            "agent.tool_denied",
                            &message,
                        )),
                        is_error: true,
                        error_code: Some("agent.tool_denied".to_string()),
                        resource_refs: Vec::new(),
                    },
                    AgentToolEffect::None,
                )
            }
        };

        Ok(AgentToolDispatchOutcome {
            result: outcome.0,
            effect: outcome.1,
            elapsed_ms: started.elapsed().as_millis(),
        })
    }

    async fn read_run_prompt_snapshot(
        &self,
        run_id: &str,
    ) -> Result<serde_json::Value, ApplicationError> {
        let snapshot_path = WorkspacePath::parse(RUN_PROMPT_SNAPSHOT_PATH)?;
        let snapshot_file = self
            .workspace_repository
            .read_text(run_id, &snapshot_path)
            .await
            .map_err(ApplicationError::from)?;
        serde_json::from_str(&snapshot_file.text).map_err(|error| {
            ApplicationError::ValidationError(format!(
                "agent.invalid_prompt_snapshot_file: failed to parse prompt snapshot JSON: {error}"
            ))
        })
    }
}
