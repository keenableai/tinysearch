# TinyBus adapter

The adapter serves `ListTools` and `ExecuteTool` using the names and payloads
from `tinysearch-bus`. `tinybus_module::module_export!` accepts `SearchConfig`
at initialization and supports live reinitialization. The service owns provider
routing and does not log credentials, arguments, or results.

The built-in registry serves every provider in `tinysearch_bus::PROVIDERS`.
`ListTools` filters them by enabled state, route, and credential availability
on every initialization or reinitialization. Parallel runs only on the direct
route with the user's own key; a backend route never lists it. Failed calls carry
`Error::bus_message`, which prefixes classified failures with
`tinysearch.<code>: `.
