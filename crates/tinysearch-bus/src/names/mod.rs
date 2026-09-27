//! Stable `TinyBus` names for `TinySearch`.
use crate::Role;
/// Interface claimed by the module.
pub const INTERFACE: &str = "ai.tinyhumans.tinysearch.Search";
/// Object path serving the interface.
pub const OBJECT_PATH: &str = "/ai/tinyhumans/tinysearch/Search";
/// Member names.
pub mod methods {
    /// Discover currently available search tools.
    pub const LIST_TOOLS: &str = "ListTools";
    /// Execute a discovered tool.
    pub const EXECUTE_TOOL: &str = "ExecuteTool";
}
/// Tool names presented in `roles` mode, one per [`Role`].
pub mod tools {
    /// Ranked web results for a query ([`Role::Search`](crate::Role::Search)).
    pub const WEB_SEARCH: &str = "web_search_tool";
    /// Grounded, cited answer ([`Role::Answer`](crate::Role::Answer)).
    pub const WEB_ANSWER: &str = "web_answer_tool";
    /// Contents of specific URLs ([`Role::Contents`](crate::Role::Contents)).
    pub const WEB_CONTENTS: &str = "web_contents_tool";
}
/// Returns the tool name that presents `role`.
#[must_use]
pub const fn role_tool_name(role: Role) -> &'static str {
    match role {
        Role::Search => tools::WEB_SEARCH,
        Role::Answer => tools::WEB_ANSWER,
        Role::Contents => tools::WEB_CONTENTS,
    }
}
/// Returns the role a tool name presents, if it is a role tool.
#[must_use]
pub fn role_for_tool(name: &str) -> Option<Role> {
    Role::ALL.into_iter().find(|role| role_tool_name(*role) == name)
}
/// Members in interface dispatch order.
pub const METHODS: &[&str] = &[methods::LIST_TOOLS, methods::EXECUTE_TOOL];
#[cfg(test)]
mod test;
