//! Tools the assistant reaches outside this crate's own: the connectors'
//! (the user's services, through MCP), behind a trait so the assistant
//! doesn't depend on how they are reached, and builds without them.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

/// One external tool, as the model sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExternalTool {
    /// A name valid for the API (`^[a-zA-Z0-9_-]{1,64}$`), unique among all
    /// tools: the connector's id and the tool's name.
    pub name: String,
    /// The service, as the user knows it ("Gmail"), for cards and the inbox.
    pub service: String,
    pub description: String,
    pub input_schema: Value,
    /// Calling it changes something (sends, creates, deletes): it asks first.
    pub acts: bool,
}

/// A call to an external tool while the model answers: the board shows it as
/// Yoru's errand (`show_fetch`, firmware 0.11.0). `state` is `start`, `done`
/// or `failed`; the same `id` starts and ends it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Errand {
    pub id: String,
    /// The service, as the user knows it ("Gmail").
    pub source: String,
    pub state: &'static str,
}

pub type ErrandListener = std::sync::Arc<dyn Fn(Errand) + Send + Sync>;

pub trait ExternalTools: Send + Sync {
    /// The tools that are enabled now.
    fn tools(&self) -> Vec<ExternalTool>;
    /// Run one: its result as text for the model, and whether it failed.
    fn call(&self, name: &str, arguments: &Value, timeout: Duration) -> Result<(String, bool), String>;
}

/// The connectors, as the assistant sees them.
#[cfg(feature = "connectors")]
impl ExternalTools for crate::connectors::Connectors {
    fn tools(&self) -> Vec<ExternalTool> {
        crate::connectors::Connectors::tools(self)
            .into_iter()
            .map(|t| ExternalTool {
                name: t.tool.qualified_name,
                service: t.connector_name,
                description: t.tool.description.or(t.tool.title).unwrap_or_default(),
                input_schema: t.tool.input_schema,
                acts: t.tool.acts,
            })
            .collect()
    }

    fn call(&self, name: &str, arguments: &Value, timeout: Duration) -> Result<(String, bool), String> {
        crate::connectors::Connectors::call(self, name, arguments.clone(), timeout).map(|o| (o.text, o.is_error)).map_err(|e| e.to_string())
    }
}
