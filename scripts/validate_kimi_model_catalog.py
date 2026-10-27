"""Offline contract checks for the three implemented Kimi Chat templates."""
import copy
import json
from pathlib import Path
from urllib.parse import urlparse
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
EXPECTED = {
    "k-k3.1-pro": "kimi-k3.1-pro",
    "k-k3.1-pro-highspeed": "kimi-k3.1-pro-highspeed",
    "k-k3": "kimi-k3",
    "k-k2.6": "kimi-k2.6",
    "k-k2.7-code": "kimi-k2.7-code",
}
EFFORTS = ["low", "high", "max"]
PROTOCOLS = ["openai-compatible", "openai-responses"]
K2 = {"k-k2.6": (["none", "high"], 262144), "k-k2.7-code": (["high"], 262144)}


def validate(catalog, sources):
    schema = json.loads((ROOT / "resources/provider_model_catalog.schema.json").read_text())
    Draft202012Validator(schema).validate(catalog)
    assert len(catalog["models"]) == 5
    assert {m["id"] for m in catalog["models"]} == set(EXPECTED)
    for m in catalog["models"]:
        assert m["model_id"] == EXPECTED[m["id"]] and m["provider_id"] == "kimi"
        assert m["connection"]["default_base_url"] == "https://api.moonshot.cn/v1"
        assert m["connection"]["auth_handler"] == "bearer_api_key"
        dimensions = {d["id"]: d for d in m["dimensions"]}
        if m["id"] in K2:
            efforts, window = K2[m["id"]]
            assert [o["id"] for o in dimensions["reasoning.effort"]["options"]] == efforts
            assert dimensions["reasoning.effort"]["default_policy"]["official_value"] == "high"
            assert [o["id"] for o in dimensions["api.protocol"]["options"]] == ["openai-compatible"]
            assert dimensions["api.protocol"]["default_policy"]["value"] == "openai-compatible"
            assert [p["protocol"] for p in m["profiles"]] == ["openai-compatible"]
            c = m["profiles"][0]
            assert c["route"] == "/chat/completions"
            assert c["bindings"] == [{"dimension": "reasoning.effort", "handler": "kimi_k2_thinking", "path": "/thinking"}]
            assert c["function_calling"] == "supported"
            assert c["binding_status"] == "api_reference_reviewed"
            assert "thinking-mode" in c["source_refs"]
            # Parameter table: K2 rejects tool_choice=required with an error.
            assert c.get("required_tool_choice") is False
            assert m["limits"]["context_window_tokens"] == window
            assert m["limits"]["max_input_tokens"] is None
            assert m["limits"]["max_output_tokens"] == 131072
            assert m["documented_facts"]["input_modalities"][0] == "text"
            assert m["documented_facts"]["streaming"] == "supported"
            continue
        assert [o["id"] for o in dimensions["reasoning.effort"]["options"]] == EFFORTS
        assert dimensions["reasoning.effort"]["default_policy"]["official_value"] == "max"
        assert [o["id"] for o in dimensions["api.protocol"]["options"]] == PROTOCOLS
        assert dimensions["api.protocol"]["default_policy"]["value"] == "openai-compatible"
        assert [p["protocol"] for p in m["profiles"]] == PROTOCOLS
        c = m["profiles"][0]
        assert c["route"] == "/chat/completions"
        assert c["bindings"] == [{"dimension": "reasoning.effort", "handler": "kimi_chat_reasoning", "path": "/reasoning_effort"}]
        assert c["function_calling"] == "supported"
        assert c["binding_status"] == "api_reference_reviewed"
        r = m["profiles"][1]
        assert r["route"] == "/responses"
        assert r["default_base_url"] == "https://api.moonshot.cn/v1"
        assert r["bindings"] == [{"dimension": "reasoning.effort", "handler": "enum_body_field", "path": "/reasoning/effort"}]
        assert r["function_calling"] == "supported"
        assert r["binding_status"] == "api_reference_reviewed"
        assert m["limits"]["context_window_tokens"] == 1000000
        assert m["limits"]["max_input_tokens"] is None
        assert m["limits"]["max_output_tokens"] == 131072
        assert m["documented_facts"]["input_modalities"] == ["text", "image"]
        assert m["documented_facts"]["streaming"] == "supported"

    def check_refs(value):
        if isinstance(value, dict):
            for key, child in value.items():
                if key == "source_refs":
                    assert child and all(ref in sources["sources"] for ref in child)
                else:
                    check_refs(child)
        elif isinstance(value, list):
            for child in value:
                check_refs(child)
    check_refs(catalog)
    for source in sources["sources"].values():
        assert urlparse(source["url"]).hostname == "platform.kimi.com"
        assert len(source["sha256"]) == 64
        assert all(c in "0123456789abcdef" for c in source["sha256"])
    overview = sources["sources"]["models-overview"]
    assert "131072" in overview["excerpt"] and "reasoning_effort" in overview["excerpt"]
    assert "reasoning_content" in sources["sources"]["chat-api"]["excerpt"]
    responses = sources["sources"]["responses-api"]
    assert "reasoning.effort" in responses["excerpt"] and "131072" in responses["excerpt"]
    assert "https://api.moonshot.cn/v1" in sources["sources"]["api-overview"]["excerpt"]
    thinking = sources["sources"]["thinking-mode"]
    assert "thinking" in thinking["excerpt"] and "kimi-k2.6" in thinking["excerpt"]


if __name__ == "__main__":
    catalog = json.loads((ROOT / "resources/kimi_model_catalog.json").read_text())
    sources = json.loads((ROOT / "resources/kimi_model_catalog.sources.json").read_text())
    validate(catalog, sources)
    for field, value in [("model_id", "wrong"), ("provider_id", "openai")]:
        invalid = copy.deepcopy(catalog)
        invalid["models"][0][field] = value
        try:
            validate(invalid, sources)
        except (AssertionError, ValueError):
            pass
        else:
            raise AssertionError("invalid identity was accepted")
    print("PASS: Kimi schema, five template contracts, K2/K3 profiles, evidence references, negative identity checks")
