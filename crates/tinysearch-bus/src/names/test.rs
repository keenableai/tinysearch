//! Unit tests for the bus name table.

use super::{INTERFACE, METHODS, OBJECT_PATH, methods};

#[test]
fn the_object_path_is_the_interface_in_path_form() {
    let expected = format!("/{}", INTERFACE.replace('.', "/"));
    assert_eq!(OBJECT_PATH, expected);
}

#[test]
fn every_member_is_listed_exactly_once() {
    let mut sorted = METHODS.to_vec();
    sorted.sort_unstable();
    let mut deduplicated = sorted.clone();
    deduplicated.dedup();
    assert_eq!(sorted, deduplicated);
}

#[test]
fn the_method_table_holds_the_declared_members() {
    assert_eq!(METHODS, [methods::LIST_TOOLS, methods::EXECUTE_TOOL]);
}

#[test]
fn no_member_name_is_empty() {
    assert!(METHODS.iter().all(|method| !method.is_empty()));
}

#[test]
fn role_tools_have_stable_names_and_round_trip() {
    use super::{role_for_tool, role_tool_name, tools};
    use crate::Role;
    assert_eq!(role_tool_name(Role::Search), "web_search_tool");
    assert_eq!(role_tool_name(Role::Answer), "web_answer_tool");
    assert_eq!(role_tool_name(Role::Contents), "web_contents_tool");
    assert_eq!(tools::WEB_SEARCH, role_tool_name(Role::Search));
    for role in Role::ALL {
        assert_eq!(role_for_tool(role_tool_name(role)), Some(role));
    }
    assert_eq!(role_for_tool("exa_search"), None);
}
