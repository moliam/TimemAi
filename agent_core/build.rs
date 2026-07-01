fn main() {
    let resources = [
        "../resources/static_v1.md",
        "../resources/response_v1_summary.json",
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
