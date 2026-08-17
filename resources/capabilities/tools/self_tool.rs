use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::response_protocol::ParsedAction;
use crate::AgentCore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfToolPaths {
    pub space_dir: PathBuf,
    pub memory_dir: PathBuf,
    pub memory_file: PathBuf,
    pub scratch_file: PathBuf,
    pub api_audit_file: PathBuf,
    pub action_audit_file: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfToolAbout {
    pub name: String,
    pub version: String,
    pub author: String,
    pub summary: String,
    pub project: String,
    pub star_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfToolProcess {
    pub pid: u32,
    pub current_dir: PathBuf,
    pub executable: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfToolState {
    env: BTreeMap<String, String>,
    paths: SelfToolPaths,
    about: SelfToolAbout,
    process: SelfToolProcess,
}

impl SelfToolState {
    pub fn new(
        env: BTreeMap<String, String>,
        paths: SelfToolPaths,
        about: SelfToolAbout,
        process: SelfToolProcess,
    ) -> Self {
        Self {
            env,
            paths,
            about,
            process,
        }
    }

    pub(crate) fn set_env_value(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.env.insert(key.into(), value.into());
    }
}

pub(crate) fn execute_action(core: &mut AgentCore, action: &ParsedAction) -> String {
    match action.input_lower("type").as_str() {
        "path" => execute_path_action(core),
        "cwd" => execute_cwd_action(core, &action.input_raw_str("new_path")),
        "params" => execute_params_action(core),
        self_type => {
            format!("Action result: self_tool\ntype: {self_type}\nerror: unsupported_type")
        }
    }
}

fn execute_cwd_action(core: &mut AgentCore, new_path: &str) -> String {
    match core.change_prompt_cwd(new_path) {
        Ok(path) => format!(
            "Action result: self_tool\ntype: cwd\nCWD changed to {}",
            path.display()
        ),
        Err(error) => {
            format!("Action result: self_tool\ntype: cwd\nerror: {error}")
        }
    }
}

fn execute_path_action(core: &AgentCore) -> String {
    let config_root = crate::default_config_root();
    let reminder_file = crate::reminder_tips_config_path(&config_root);
    let data_root = core
        .self_tool
        .paths
        .space_dir
        .parent()
        .unwrap_or(&core.self_tool.paths.space_dir);
    let audit_dir = core
        .self_tool
        .paths
        .api_audit_file
        .parent()
        .unwrap_or(&core.self_tool.paths.space_dir);
    let workspace_config_file = crate::workspace_config_file(data_root);
    let sessions_dir = core.self_tool.paths.memory_dir.join("sessions");
    let session_index_file = sessions_dir.join("index.jsonl");
    let tool_repo_dir = core.tool_repo().root();
    let capabilities_dir = env_path_param(&core.self_tool.env, "TIMEM_CAPABILITIES_DIR");
    format!(
        "Action result: self_tool\ntype: path\ncwd: {}\nprocess_cwd: {}\nexecutable: {}\nconfig_root: {}\nreminder_tips_file: {}\ncapabilities_dir: {}\ndata_root: {}\nworkspace_config_file: {}\nspace_dir: {}\nmemory_dir: {}\nmemory_file: {}\nscratch_file: {}\nsessions_dir: {}\nsession_index_file: {}\ntool_repo_dir: {}\naudit_dir: {}\napi_audit_file: {}\naction_audit_file: {}",
        core.current_prompt_cwd().display(),
        core.self_tool.process.current_dir.display(),
        core.self_tool.process.executable.display(),
        config_root.display(),
        reminder_file.display(),
        capabilities_dir,
        data_root.display(),
        workspace_config_file.display(),
        core.self_tool.paths.space_dir.display(),
        core.self_tool.paths.memory_dir.display(),
        core.self_tool.paths.memory_file.display(),
        core.self_tool.paths.scratch_file.display(),
        sessions_dir.display(),
        session_index_file.display(),
        tool_repo_dir.display(),
        audit_dir.display(),
        core.self_tool.paths.api_audit_file.display(),
        core.self_tool.paths.action_audit_file.display(),
    )
}

fn execute_params_action(core: &AgentCore) -> String {
    let safe_env = &core.self_tool.env;
    format!(
        "Action result: self_tool\ntype: params\nname: {}\nversion: {}\npid: {}\nmodel: {}\nassistant_name: {}\napi_protocol: {}\nresponse_protocol: {}\nbase_url: {}\napi_key_configured: {}\ntimeout_secs: {}\nmax_llm_input_tokens: {}\nmax_llm_output_tokens: {}\nmax_steps: {}\nbash_approval: {}\nwork_instructions: {}\nenable_thinking: {}\nreasoning_effort: {}\nstream: {}\nopenai_cache_mode: {}\ncapability_tools: {}\ncapability_skills: {}\nnote: Only known runtime parameters are returned. Credentials and arbitrary environment variables are excluded.",
        core.self_tool.about.name,
        core.self_tool.about.version,
        core.self_tool.process.pid,
        env_param_or(safe_env, "TIMEM_MODEL", &core.profile().model),
        core.assistant_speaker_name(),
        env_param(safe_env, "TIMEM_API_PROTOCOL"),
        core.response_protocol_name(),
        safe_base_url_param(safe_env),
        safe_env
            .get("TIMEM_API_KEY")
            .is_some_and(|value| !value.trim().is_empty()),
        env_param(safe_env, "TIMEM_TIMEOUT"),
        core.max_llm_input_tokens(),
        env_param(safe_env, "TIMEM_MAX_LLM_OUTPUT"),
        round_budget_label(core.configured_round_budget()),
        crate::bash_approval_mode_label(core.bash_approval_mode),
        env_param(safe_env, "TIMEM_WORK_INSTRUCTIONS"),
        env_param(safe_env, "TIMEM_ENABLE_THINKING"),
        env_param(safe_env, "TIMEM_REASONING_EFFORT"),
        env_param(safe_env, "TIMEM_STREAM"),
        env_param(safe_env, "TIMEM_OPENAI_CACHE_MODE"),
        core.capabilities.tool_count(),
        core.capabilities.skill_count(),
    )
}

fn env_path_param(env: &BTreeMap<String, String>, key: &str) -> String {
    env.get(key)
        .map(|value| PathBuf::from(value).display().to_string())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "not_configured".to_string())
}

fn env_param(env: &BTreeMap<String, String>, key: &str) -> String {
    env.get(key)
        .and_then(|value| serde_json::to_string(value).ok())
        .unwrap_or_else(|| "null".to_string())
}

fn env_param_or(env: &BTreeMap<String, String>, key: &str, fallback: &str) -> String {
    let value = env.get(key).map(String::as_str).unwrap_or(fallback);
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

fn safe_base_url_param(env: &BTreeMap<String, String>) -> String {
    let Some(value) = env.get("TIMEM_BASE_URL") else {
        return "null".to_string();
    };
    let without_fragment = value.split('#').next().unwrap_or(value);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    let sanitized = if let Some((scheme, rest)) = without_query.split_once("://") {
        let authority_end = rest.find('/').unwrap_or(rest.len());
        let (authority, path) = rest.split_at(authority_end);
        let host = authority
            .rsplit_once('@')
            .map(|(_, host)| host)
            .unwrap_or(authority);
        format!("{scheme}://{host}{path}")
    } else {
        without_query.to_string()
    };
    serde_json::to_string(&sanitized).unwrap_or_else(|_| "null".to_string())
}

fn round_budget_label(rounds: u32) -> String {
    if rounds == crate::UNLIMITED_ROUND_BUDGET {
        "unlimited".to_string()
    } else {
        rounds.to_string()
    }
}

#[cfg(test)]
#[path = "../../../agent_core/tests/unit/capability_tool_self_tool_tests.rs"]
mod tests;
