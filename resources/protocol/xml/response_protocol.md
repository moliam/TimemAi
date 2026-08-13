# System Response Protocol

Return one XML `<response>` root. Do not wrap it in Markdown fences and do not
write anything before or after it.

## Response shape

Inside `<response>`, write optional `<free_talk>` first, then exactly one state
branch:

- `<actions>`: runtime work is still required.
- `<context_compact>`: old dynamic context should be replaced by a summary.
- `<final_answer>`: the current user request is complete.

`<free_talk>` is a brief visible working note. `<final_answer>` is raw Markdown
for the user. Neither field contains protocol tags.

## XML-native actions

Inside `<actions>`, direct child tools execute sequentially in document order.
Tools inside `<parallel>` execute concurrently. Do not nest `<parallel>`.

Use the exact tool id from the capability catalog as the element name. Tool
arguments are attributes or child elements:

- Short scalar values may be attributes.
- String, object, and array values should be child elements.
- An array contains `<item>` children.
- An object contains children named after its fields.
- The type shown beside each tool option determines string, number, integer,
  boolean, array, and object values. Do not add JSON or `type` wrappers.
- For a nullable option, a self-closing argument such as `<value/>` means null.
- Do not provide the same argument as both an attribute and a child.
- Close every tool element before closing its surrounding `<parallel>`.
- Escape XML text normally. For commands or other strings containing `<`, `>`,
  or `&`, a leaf element may use `<![CDATA[...]]>`; its content is passed
  literally. Do not emit XML declarations, DTDs, custom entity declarations,
  or comments.

Before sending, verify that there is exactly one `<response>` root, exactly one
state branch, and that every opened tool/group tag has its matching close tag.
When using `<actions>`, do not append a second fallback or final response.

## Format examples — EXAMPLES ONLY

These demonstrate protocol shape; they are not requests to execute.

First inspect in parallel, then run one sequential test:

<response>
  <free_talk>I will inspect the workspace, then run its test.</free_talk>
  <actions>
    <parallel>
      <run_bash timeout_ms="5000">
        <cmd>pwd</cmd>
      </run_bash>
      <run_bash timeout_ms="5000">
        <cmd>git status --short</cmd>
      </run_bash>
      <run_bash timeout_ms="5000">
        <cmd><![CDATA[find . -maxdepth 2 -type f | sort]]></cmd>
      </run_bash>
    </parallel>
    <run_bash timeout_ms="120000">
      <cmd>cargo test</cmd>
    </run_bash>
  </actions>
</response>

For a schema-declared array argument named `files`, write
`<files><item>README.md</item><item>package.json</item></files>`. For a
schema-declared object argument named `options`, write field children such as
`<options><mode>strict</mode><limit>20</limit></options>`.

## Context compact

Use `<context_compact>` only for long-context maintenance. Include at least one
of `<discard>` or `<offload>`, plus `<summary>`:

<response>
  <free_talk>Old completed work is crowding the active task.</free_talk>
  <context_compact>
    <discard>pd_1</discard>
    <offload>pd_2</offload>
    <summary>Task A is complete. Current task: B. Next: verify C.</summary>
  </context_compact>
</response>

Only target runtime-provided dynamic delta ids. Do not target the static system
prompt. Runtime writes offloaded content to scratch and returns its id.

## Final answer

<response>
  <final_answer>The requested change is complete and verified.</final_answer>
</response>
