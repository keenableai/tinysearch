# TinySearch module

The `tinysearch` native module implements TinyBus ABI v1 and serves the
`ai.tinyhumans.tinysearch.Search` interface at
`/ai/tinyhumans/tinysearch/Search`.

`ListTools` returns tools available under the current configuration.
`ExecuteTool` accepts a `name` from that list and provider-specific JSON
`arguments`. Module initialization and reinitialization carry the sensitive
`SearchConfig`, including credentials; tool arguments must never carry those
credentials. Reinitialization replaces the connection and service state.

By default `ListTools` presents capability roles (`web_search_tool`,
`web_answer_tool`, `web_contents_tool`), each dispatched across an ordered
provider list with fallback. Classified failures cross the bus as
`tinysearch.<code>: <message>`; see `tinysearch_bus::errors`.

The typed wire contract, version 2.0, is in `tinysearch-bus`.
