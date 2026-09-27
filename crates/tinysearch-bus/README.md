# tinysearch-bus

This crate defines TinySearch's TinyBus names, configuration, tool declarations,
and normalized results. It has no runtime or transport dependency. The
`provider_tool_specs` catalog and `select_tools` presentation function are pure
and usable by a host synchronously.

`BackendConfig.auth_mode` distinguishes session bearer credentials from API keys.
`SearchStatus::InProgress` signals asynchronous research that can be resumed
using the returned `interaction_id` in `provider_data`.

Contract version 2.0 adds capability roles. `Role` names a capability
(`search`, `answer`, `contents`); `PresentationMode::Roles` (the default)
presents one tool per role, named by `role_tool_name`. `provider_roles`,
`default_role_providers`, and `role_providers` describe which providers serve a
role and in which order; `PresentationConfig.roles` overrides the order.
`ExecuteToolResponse.role` and `fallback_from` report how a role call was
served. `PROVIDERS` lists every provider and `BACKEND_PROVIDERS` those with a
managed backend route. The `errors` module holds the stable failure codes
carried in bus error messages and `code_of` to read them.
