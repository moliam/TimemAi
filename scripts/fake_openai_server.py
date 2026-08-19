#!/usr/bin/env python3
import argparse
import json
import re
import shlex
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from xml.sax.saxutils import escape


FINISH_CONFIRM_PREFIX = (
    "Now let me think seriously twice before I announce stop. Review user's task "
    "list. Is my delivery consistent with user's demand?"
)


def extract_text(value):
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        parts = []
        for item in value:
            if isinstance(item, dict):
                parts.append(extract_text(item.get("text", "")))
            else:
                parts.append(extract_text(item))
        return "\n".join(part for part in parts if part)
    if isinstance(value, dict):
        return extract_text(value.get("text", ""))
    return ""


def extract_prompt(body):
    messages = body.get("messages", [])
    if messages:
        return "\n".join(extract_text(message.get("content", "")) for message in messages)
    parts = [extract_text(body.get("instructions", "")), extract_text(body.get("input", ""))]
    return "\n".join(part for part in parts if part)


class Handler(BaseHTTPRequestHandler):
    response_delay = 0.0
    capture_prompt_file = None
    scenario = "default"

    def log_message(self, _format, *_args):
        return

    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        try:
            body = json.loads(self.rfile.read(length) or b"{}")
        except json.JSONDecodeError:
            self.send_json(400, {"error": "invalid_json"})
            return

        prompt = extract_prompt(body)
        if self.capture_prompt_file:
            with open(self.capture_prompt_file, "a", encoding="utf-8") as capture:
                capture.write(prompt)
                capture.write("\n---TIMEM_FAKE_MODEL_REQUEST---\n")
        if self.scenario == "toolgen":
            content = toolgen_scenario_response(prompt)
        elif "CROSS_HOST_RESUME_SMOKE" in prompt:
            content = (
                "<ASSISTANT>"
                "<final_answer>CROSS_HOST_RESUME_OK</final_answer>"
                "</ASSISTANT>"
            )
        elif "TTY_STRESS" in prompt and "STRESS_ACTION_DONE" in prompt:
            content = (
                "<ASSISTANT>"
                "<final_answer>STRESS_OK</final_answer>"
                "</ASSISTANT>"
            )
        elif "TTY_STRESS" in prompt:
            time.sleep(self.response_delay)
            content = tty_stress_scenario_response()
        elif "## USER" in prompt and "SUPPLEMENT_OK" in prompt:
            content = (
                "<ASSISTANT>"
                "<final_answer>SUPPLEMENT_OK</final_answer>"
                "</ASSISTANT>"
            )
        else:
            time.sleep(self.response_delay)
            content = (
                "<ASSISTANT>"
                "<final_answer>NO_SUPPLEMENT</final_answer>"
                "</ASSISTANT>"
            )

        self.send_model_response(prompt, content)

    def send_model_response(self, prompt, content):
        content = confirm_xml_final(content)
        prompt_tokens = max(1, len(prompt) // 4)
        completion_tokens = max(1, len(content) // 4)
        total_tokens = max(2, (len(prompt) + len(content)) // 4)
        if self.path.rstrip("/").endswith("/responses"):
            self.send_json(
                200,
                {
                    "output_text": content,
                    "usage": {
                        "input_tokens": prompt_tokens,
                        "output_tokens": completion_tokens,
                        "total_tokens": total_tokens,
                        "input_tokens_details": {"cached_tokens": 0},
                        "output_tokens_details": {"reasoning_tokens": 0},
                    },
                },
            )
            return
        self.send_json(
            200,
            {
                "choices": [{"message": {"content": content}, "finish_reason": "stop"}],
                "usage": {
                    "prompt_tokens": prompt_tokens,
                    "completion_tokens": completion_tokens,
                    "total_tokens": total_tokens,
                },
            },
        )

    def send_json(self, status, payload):
        encoded = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def confirm_xml_final(content):
    if "<final_answer" not in content or "<finish_confirm" in content:
        return content
    confirmation = f"<finish_confirm>{FINISH_CONFIRM_PREFIX}</finish_confirm>"
    candidates = [
        pos
        for pos in (content.find("<toolgen_retrospect"), content.find("<final_answer"))
        if pos >= 0
    ]
    insertion_point = min(candidates)
    return content[:insertion_point] + confirmation + content[insertion_point:]


def xml_element(name, value):
    if value is None:
        return f"<{name}/>"
    if isinstance(value, bool):
        text = "true" if value else "false"
        return f"<{name}>{text}</{name}>"
    if isinstance(value, dict):
        body = "".join(xml_element(child, child_value) for child, child_value in value.items())
        return f"<{name}>{body}</{name}>"
    if isinstance(value, list):
        body = "".join(xml_element("item", item) for item in value)
        return f"<{name}>{body}</{name}>"
    return f"<{name}>{escape(str(value))}</{name}>"


def xml_action(payload, free_talk):
    if len(payload) != 1:
        raise ValueError("XML action fixture requires exactly one tool")
    tool, arguments = next(iter(payload.items()))
    action_name = escape(f"run fake {tool} action")
    action_body = xml_element(tool, arguments)
    action_body = action_body.replace(f"<{tool}>", f'<{tool} name="{action_name}">', 1)
    return (
        "<ASSISTANT>"
        f"<free_talk>{free_talk}</free_talk>"
        "<actions>"
        + action_body
        + "</actions>"
        "</ASSISTANT>"
    )



def tty_stress_scenario_response():
    free_talk = (
        "正在执行真实终端压力测试：验证 Thought / Action 面板在长进度、"
        "长 Bash 命令、CJK 字符、box drawing 字符 │└─、以及用户中途补充"
        "同时出现时仍然能稳定换行、保持边框宽度，并且不会重复残留旧行。"
    )
    return xml_action(
        {
            "run_bash": {
                "cmd": (
                    "printf 'STRESS_ACTION_DONE\\n'; "
                    "sleep 1; "
                    "printf '长输出-一二三四五六七八九十-abcdefghijklmnopqrstuvwxyz-1234567890-│└─\\n'"
                ),
                "timeout_ms": 5000,
            },
        },
        free_talk,
    )


def toolgen_scenario_response(prompt):
    if "[TOOL_GEN_TASK]" not in prompt:
        if "Action result: run_bash" in prompt and "TOOLGEN_E2E_SOURCE_DONE" in prompt:
            return (
                "<ASSISTANT><free_talk>The reusable source task completed.</free_talk>"
                "<final_answer>ToolGen source task completed with two ERROR records.</final_answer>"
                "</ASSISTANT>"
            )
        return xml_action(
            {
                "run_bash": {
                    "cmd": "printf 'INFO ready\\nERROR alpha\\nERROR beta\\n' | grep -c ERROR; printf TOOLGEN_E2E_SOURCE_DONE",
                    "timeout_ms": 5000,
                }
            },
            "Running a deterministic source task before ToolGen.",
        )

    marker = "Write the new tool files only in this temporary staging directory:\n"
    match = re.search(re.escape(marker) + r"([^\n]+)", prompt)
    if not match:
        return "<ASSISTANT><final_answer>ToolGen fixture could not locate its draft.</final_answer></ASSISTANT>"
    draft = match.group(1).strip()
    if "FORCE_TOOLGEN_PROTOCOL_FAILURE" in prompt:
        return "ToolGen fixture intentionally returned a non-protocol response."
    if "Action result: toolgen\nop: publish\nstatus: ready" in prompt:
        return (
            "<ASSISTANT><toolgen_retrospect>Published deterministic-log-error-counter after runtime validation.</toolgen_retrospect>"
            "<final_answer>ToolGen generated and validated deterministic-log-error-counter.</final_answer>"
            "</ASSISTANT>"
        )
    if "Action result: run_bash" in prompt and "TOOLGEN_E2E_DRAFT_READY" in prompt:
        return xml_action(
            {"toolgen": {"op": "publish", "draft_path": draft}},
            "The draft is ready; asking runtime to validate and publish it.",
        )

    readme = (
        "# deterministic-log-error-counter\n\n"
        "Count ERROR records in a text log.\n"
        "Synopsis: `count-errors.sh <log-file>`\n"
        "Prerequisites: bash and grep.\n"
        "Input: one readable log path. Output: one integer.\n"
        "Example: `./count-errors.sh build.log`\n"
    )
    script = (
        "#!/usr/bin/env bash\nset -euo pipefail\n"
        "if [[ ${1:-} == --self-test ]]; then "
        "sample=$(mktemp); trap 'rm -f \"$sample\"' EXIT; "
        "printf 'INFO\\nERROR one\\nERROR two\\n' > \"$sample\"; "
        "[[ $(grep -c ERROR \"$sample\") == 2 ]]; echo ready; exit 0; fi\n"
        "[[ $# -eq 1 ]] || { echo 'usage: count-errors.sh <log-file>' >&2; exit 2; }\n"
        "grep -c ERROR \"$1\" || [[ $? -eq 1 ]]\n"
    )
    manifest = json.dumps(
        {
            "name": "deterministic-log-error-counter",
            "type": "log-analysis",
            "language": "bash",
            "entrypoint": "count-errors.sh",
            "synopsis": "count-errors.sh <log-file>",
            "self_test": {"args": ["--self-test"], "timeout_ms": 5000},
        },
        separators=(",", ":"),
    )
    command = " && ".join(
        [
            f"mkdir -p {shlex.quote(draft)}",
            f"printf %s {shlex.quote(readme)} > {shlex.quote(draft + '/README.md')}",
            f"printf %s {shlex.quote(script)} > {shlex.quote(draft + '/count-errors.sh')}",
            f"printf %s {shlex.quote(manifest)} > {shlex.quote(draft + '/.timem-tool.json')}",
            "printf TOOLGEN_E2E_DRAFT_READY",
        ]
    )
    return xml_action(
        {"run_bash": {"cmd": command, "timeout_ms": 5000}},
        "Creating the reusable ToolGen draft and its deterministic self-test.",
    )


def self_test():
    assert extract_prompt({"messages": [{"content": "hello"}]}) == "hello"
    assert extract_prompt({"instructions": "system", "input": "user"}) == "system\nuser"
    stress = tty_stress_scenario_response()
    assert "<actions>" in stress and '<run_bash name="run fake run_bash action">' in stress
    assert "STRESS_ACTION_DONE" in stress
    assert "<working_still_action>" not in stress
    assert "<action_json>" not in stress
    source = toolgen_scenario_response("TOOLGEN_E2E_SOURCE")
    assert '<run_bash name="' in source and "TOOLGEN_E2E_SOURCE_DONE" in source
    completed = toolgen_scenario_response(
        "Action result: run_bash\noutput: TOOLGEN_E2E_SOURCE_DONE"
    )
    assert "ToolGen source task completed" in completed
    toolgen_prompt = (
        "[TOOL_GEN_TASK]\n"
        "Write the new tool files only in this temporary staging directory:\n"
        "/tmp/toolgen-fixture-draft\n"
    )
    draft = toolgen_scenario_response(toolgen_prompt)
    assert '<run_bash name="' in draft and "TOOLGEN_E2E_DRAFT_READY" in draft
    publish = toolgen_scenario_response(
        toolgen_prompt + "\nAction result: run_bash\noutput: TOOLGEN_E2E_DRAFT_READY"
    )
    assert '<toolgen name="' in publish and "<op>publish</op>" in publish
    finished = toolgen_scenario_response(
        toolgen_prompt + "\nAction result: toolgen\nop: publish\nstatus: ready"
    )
    assert "toolgen_retrospect" in finished and "final_answer" in finished
    failed = toolgen_scenario_response(toolgen_prompt + "\nFORCE_TOOLGEN_PROTOCOL_FAILURE")
    assert failed == "ToolGen fixture intentionally returned a non-protocol response."
    print("fake_model_server_toolgen_scenario: ok")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--delay", type=float, default=2.0)
    parser.add_argument("--capture-prompt-file")
    parser.add_argument("--scenario", choices=("default", "toolgen"), default="default")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    Handler.response_delay = args.delay
    Handler.capture_prompt_file = args.capture_prompt_file
    Handler.scenario = args.scenario
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"fake_model_server_ready:{server.server_port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
