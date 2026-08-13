Return exactly one XML `<response>` root with no outer Markdown fence or extra
text. Inside it, write optional `<free_talk>` first, followed by exactly one of:

- `<actions>` when runtime tools are needed.
- `<context_compact>` for dynamic-context maintenance.
- `<final_answer>` when the current request is complete.

`<actions>` is XML-native. Direct child tool elements execute sequentially in
document order. Tool elements inside one `<parallel>` execute concurrently;
`<parallel>` cannot be nested. The tool element name is the exact capability id.

Arguments may be short scalar attributes or child elements. Arrays contain
`<item>` children; objects contain children named after their fields. Types shown
beside tool options determine scalar and nested values. Never encode actions or
arguments as JSON. Never duplicate an argument between an attribute and child.
For a nullable option, a self-closing argument element means null.
Leaf string elements may use CDATA for commands or text containing XML-special
characters; CDATA content is literal. XML declarations, DTDs, custom entities,
and comments are not part of this protocol.

`<context_compact>` contains `<discard>` and/or `<offload>`, plus `<summary>`.
Ids are comma-separated dynamic delta ids. `<final_answer>` is raw user-facing
Markdown. `<free_talk>`, `<summary>`, and `<final_answer>` are text, not nested
response protocol.
