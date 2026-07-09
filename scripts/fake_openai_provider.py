#!/usr/bin/env python3
import argparse
import json
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


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


class Handler(BaseHTTPRequestHandler):
    response_delay = 0.0

    def log_message(self, _format, *_args):
        return

    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        try:
            body = json.loads(self.rfile.read(length) or b"{}")
        except json.JSONDecodeError:
            self.send_json(400, {"error": "invalid_json"})
            return

        messages = body.get("messages", [])
        prompt = "\n".join(extract_text(message.get("content", "")) for message in messages)
        if "TTY_STRESS" in prompt and "STRESS_ACTION_DONE" in prompt:
            content = "## Status\nfinished\n\n## Final_Answer\nSTRESS_OK"
        elif "TTY_STRESS" in prompt:
            time.sleep(self.response_delay)
            free_talk = (
                "正在执行真实终端压力测试：验证 Thought / Action 面板在长进度、"
                "长 Bash 命令、CJK 字符、box drawing 字符 │└─、以及用户中途补充"
                "同时出现时仍然能稳定换行、保持边框宽度，并且不会重复残留旧行。"
            )
            content = (
                "## Free_talk\n"
                + free_talk
                + "\n\n## Working_Still_Action\n```action\n"
                + json.dumps(
                    {
                        "action": "run_bash",
                        "args": {
                            "cmd": (
                                "printf 'STRESS_ACTION_DONE\\n'; "
                                "sleep 1; "
                                "printf '长输出-一二三四五六七八九十-abcdefghijklmnopqrstuvwxyz-1234567890-│└─\\n'"
                            ),
                            "timeout_ms": 5000,
                        },
                    },
                    ensure_ascii=False,
                )
                + "\n```"
            )
        elif "## USER" in prompt and "SUPPLEMENT_OK" in prompt:
            content = "## Status\nfinished\n\n## Final_Answer\nSUPPLEMENT_OK"
        else:
            time.sleep(self.response_delay)
            content = "## Status\nfinished\n\n## Final_Answer\nNO_SUPPLEMENT"

        self.send_json(
            200,
            {
                "choices": [{"message": {"content": content}, "finish_reason": "stop"}],
                "usage": {
                    "prompt_tokens": max(1, len(prompt) // 4),
                    "completion_tokens": max(1, len(content) // 4),
                    "total_tokens": max(2, (len(prompt) + len(content)) // 4),
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


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--delay", type=float, default=2.0)
    args = parser.parse_args()
    Handler.response_delay = args.delay
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"fake_provider_ready:{server.server_port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
