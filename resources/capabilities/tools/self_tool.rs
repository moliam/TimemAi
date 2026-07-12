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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelfToolInput<'a> {
    pub self_type: &'a str,
    pub op: &'a str,
    pub key: &'a str,
    pub value: &'a str,
    pub new_path: &'a str,
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

    pub fn execute(&mut self, input: SelfToolInput<'_>) -> String {
        if input.self_type.trim().is_empty() {
            return "Action result: self_tool\nerror: invalid_input\nmessage: Missing `type`. Use env, mem_path, about_me, or cwd.".to_string();
        }
        if input.op.trim().is_empty() {
            return format!(
                "Action result: self_tool\ntype: {}\nerror: invalid_input\nmessage: Missing `op`. Use read, write, or chg_cwd.",
                input.self_type
            );
        }
        match (input.self_type, input.op) {
            ("env", "read") => self.read_env(input.key),
            ("env", "write") => self.write_env(input.key, input.value),
            ("mem_path", "read") => self.read_mem_paths(),
            ("about_me", "read") => self.read_about(),
            (self_type, op) => format!(
                "Action result: self_tool\ntype: {self_type}\nop: {op}\nerror: unsupported_type_or_op"
            ),
        }
    }

    fn read_env(&self, key: &str) -> String {
        let key = key.trim();
        if !key.is_empty() {
            if is_sensitive_env_key(key) {
                return format!(
                    "Action result: self_tool\ntype: env\nop: read\nkey: {key}\nerror: sensitive_env_denied"
                );
            }
            let value = self
                .env
                .get(key)
                .cloned()
                .or_else(|| std::env::var(key).ok());
            return match value {
                Some(value) => format!(
                    "Action result: self_tool\ntype: env\nop: read\nkey: {key}\nvalue: {value}"
                ),
                None => format!(
                    "Action result: self_tool\ntype: env\nop: read\nkey: {key}\nfound: false"
                ),
            };
        }

        let rows = self
            .env
            .iter()
            .filter(|(key, _)| !is_sensitive_env_key(key))
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>();
        if rows.is_empty() {
            "Action result: self_tool\ntype: env\nop: read\nresults: none".to_string()
        } else {
            format!(
                "Action result: self_tool\ntype: env\nop: read\nresults:\n{}",
                rows.join("\n")
            )
        }
    }

    fn write_env(&mut self, key: &str, value: &str) -> String {
        let key = key.trim();
        if key.is_empty() {
            return "Action result: self_tool\ntype: env\nop: write\nerror: key_required"
                .to_string();
        }
        if is_sensitive_env_key(key) {
            return format!(
                "Action result: self_tool\ntype: env\nop: write\nkey: {key}\nerror: sensitive_env_denied"
            );
        }
        if is_memory_path_env_key(key) {
            return format!(
                "Action result: self_tool\ntype: env\nop: write\nkey: {key}\nerror: protected_env_denied\nreason: memory_path_env_is_startup_only"
            );
        }
        self.env.insert(key.to_string(), value.to_string());
        std::env::set_var(key, value);
        format!(
            "Action result: self_tool\ntype: env\nop: write\nkey: {key}\nstatus: updated_current_process_env\nnote: Model/provider config changes may still require /config or restart to take effect."
        )
    }

    fn read_mem_paths(&self) -> String {
        format!(
            "Action result: self_tool\ntype: mem_path\nop: read\nspace_dir: {}\nmemory_dir: {}\nmemory_file: {}\nscratch_file: {}\napi_audit_file: {}\naction_audit_file: {}",
            self.paths.space_dir.display(),
            self.paths.memory_dir.display(),
            self.paths.memory_file.display(),
            self.paths.scratch_file.display(),
            self.paths.api_audit_file.display(),
            self.paths.action_audit_file.display()
        )
    }

    fn read_about(&self) -> String {
        format!(
            "Action result: self_tool\ntype: about_me\nop: read\nname: {}\nversion: {}\nauthor: {}\nsummary: {}\nproject: {}\nstar_message: {}\npid: {}\ncurrent_dir: {}\nexecutable: {}",
            self.about.name,
            self.about.version,
            self.about.author,
            self.about.summary,
            self.about.project,
            self.about.star_message,
            self.process.pid,
            self.process.current_dir.display(),
            self.process.executable.display()
        )
    }
}

pub(crate) fn execute_action(core: &mut AgentCore, action: &ParsedAction) -> String {
    let self_type = action.input_lower("type");
    let op = action.input_lower("op");
    if self_type == "cwd" {
        return execute_cwd_action(core, &op, &action.input_str("new_path"));
    }
    core.self_tool.execute(SelfToolInput {
        self_type: &self_type,
        op: &op,
        key: &action.input_str("key"),
        value: &action.input_raw_str("value"),
        new_path: &action.input_raw_str("new_path"),
    })
}

fn execute_cwd_action(core: &mut AgentCore, op: &str, new_path: &str) -> String {
    match op {
        "read" => format!(
            "Action result: self_tool\ntype: cwd\nop: read\ncwd: {}",
            core.current_prompt_cwd().display()
        ),
        "chg_cwd" => match core.change_prompt_cwd(new_path) {
            Ok(path) => format!(
                "Action result: self_tool\ntype: cwd\nop: chg_cwd\nstatus: updated_prompt_context_cwd\ncwd: {}\nnote: Future run_bash actions in this prompt context will execute from this cwd.",
                path.display()
            ),
            Err(error) => format!(
                "Action result: self_tool\ntype: cwd\nop: chg_cwd\nerror: {error}\nnew_path: {}",
                new_path.trim()
            ),
        },
        other => format!("Action result: self_tool\ntype: cwd\nop: {other}\nerror: unsupported_type_or_op"),
    }
}

pub fn is_sensitive_env_key(key: &str) -> bool {
    let compact = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    compact.contains("APIKEY")
        || compact.contains("ACCESSKEY")
        || compact.contains("SECRET")
        || compact.contains("PASSWORD")
        || compact.contains("CREDENTIAL")
        || compact == "KEY"
        || compact.ends_with("TOKEN")
}

pub fn is_memory_path_env_key(key: &str) -> bool {
    let compact = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    matches!(
        compact.as_str(),
        "TIMEMSPACE"
            | "TIMEMDATADIR"
            | "TIMEMDATAROOT"
            | "TIMEMMEMPATH"
            | "TIMEMMEMORYPATH"
            | "TIMEMMEMORYDIR"
            | "TIMEMMEMORYROOT"
            | "TIMEMAUDITPATH"
            | "TIMEMAUDITDIR"
            | "TIMEMAUDITROOT"
    )
}

#[cfg(test)]
#[path = "../../../agent_core/tests/unit/capability_tool_self_tool_tests.rs"]
mod tests;
