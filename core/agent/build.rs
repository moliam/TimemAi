fn main() {
    let resources = [
        "../../resources/system_prompt/system_prompt.md",
        "../../resources/reminder_tips.json",
        "../../resources/protocol/json/response_protocol.md",
        "../../resources/protocol/json/response_schema_summary.json",
        "../../resources/protocol/xml/response_protocol.md",
        "../../resources/capabilities/tools/capmgr.yaml",
        "../../resources/capabilities/tools/memmgr.yaml",
        "../../resources/capabilities/tools/readfile.yaml",
        "../../resources/capabilities/tools/run_bash.yaml",
        "../../resources/capabilities/tools/self_tool.yaml",
        "../../resources/capabilities/tools/toolgen.yaml",
    ];

    for resource in resources {
        println!("cargo:rerun-if-changed={resource}");
    }
}
