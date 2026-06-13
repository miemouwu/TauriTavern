use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::watch;

use crate::application::errors::ApplicationError;
use crate::application::services::chat_completion_service::ChatCompletionService;
use crate::domain::models::agent::{AgentModelRequest, AgentModelResponse};
use crate::domain::repositories::tokenizer_repository::TokenizerRepository;
use crate::infrastructure::logging::logger;

mod decode;
mod encode;
mod format;
mod model_context;
mod provider_state;
mod providers;
mod schema;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use decode::decode_chat_completion_response;

#[async_trait]
pub trait AgentModelGateway: Send + Sync {
    async fn generate_with_cancel(
        &self,
        request: AgentModelRequest,
        cancel: watch::Receiver<bool>,
    ) -> Result<AgentModelExchange, ApplicationError>;

    async fn close_session(&self, session_id: &str);
}

#[derive(Debug, Clone)]
pub struct AgentModelExchange {
    pub response: AgentModelResponse,
    pub provider_state: Value,
}

pub struct ChatCompletionAgentModelGateway {
    chat_completion_service: Arc<ChatCompletionService>,
    tokenizer: Arc<dyn TokenizerRepository>,
}

impl ChatCompletionAgentModelGateway {
    pub fn new(
        chat_completion_service: Arc<ChatCompletionService>,
        tokenizer: Arc<dyn TokenizerRepository>,
    ) -> Self {
        Self {
            chat_completion_service,
            tokenizer,
        }
    }
}

#[async_trait]
impl AgentModelGateway for ChatCompletionAgentModelGateway {
    async fn generate_with_cancel(
        &self,
        request: AgentModelRequest,
        cancel: watch::Receiver<bool>,
    ) -> Result<AgentModelExchange, ApplicationError> {
        let dto = encode::encode_chat_completion_request(&request)?;
        // Agent Memory P0: measure the encoded provider payload against the model's window.
        if let (Some(model), Some(messages)) = (
            dto.payload.get("model").and_then(|v| v.as_str()),
            dto.payload.get("messages").and_then(|v| v.as_array()),
        ) {
            if let Ok(tokens) = self.tokenizer.count_messages(model, messages) {
                let budget =
                    model_context::PromptBudget::new(tokens, model_context::max_context_for(model));
                logger::debug(&format!(
                    "agent prompt budget: model={} tokens={} max={} ratio={:.3}",
                    model, budget.tokens, budget.max_context, budget.ratio
                ));
            }
        }
        let exchange = self
            .chat_completion_service
            .generate_exchange_with_cancel(dto, cancel)
            .await?;
        let source = exchange.source;
        let adapter = providers::AgentProviderAdapter::from_format(exchange.provider_format);
        let response = decode::decode_chat_completion_exchange(exchange, &request.tools)?;
        let provider_state =
            provider_state::next_provider_state(&request, source, adapter, &response)?;

        Ok(AgentModelExchange {
            response,
            provider_state,
        })
    }

    async fn close_session(&self, session_id: &str) {
        self.chat_completion_service
            .close_provider_session(session_id)
            .await;
    }
}
