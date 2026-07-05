fn main() {
    let resources = [
        "../resources/system_prompt/system_prompt.md",
        "../resources/protocol/json/response_protocol.md",
        "../resources/protocol/json/response_schema_summary.json",
        "../resources/protocol/markdown/response_protocol.md",
        "../resources/protocol/markdown/response_schema_summary.md",
        "../resources/capabilities/tools/capmgr.yaml",
        "../resources/capabilities/tools/memmgr.yaml",
        "../resources/capabilities/tools/run_bash.yaml",
        "../resources/capabilities/tools/self_tool.yaml",
        "../resources/capabilities/tools/shell_job_status.yaml",
    ];

    for resource in resources {
        println!("cargo:rerun-if-changed={resource}");
    }
}
