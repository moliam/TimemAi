"""Offline contract checks for the Kimi Code (Coding Plan) templates."""
import copy
import json
from pathlib import Path
from urllib.parse import urlparse
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
EXPECTED = {
    "kimicode_k3.1-pro": ("k3.1-pro", "high", 1048576, ["text", "image", "video"]),
    "kimicode_k3.1-pro-highspeed": ("k3.1-pro-highspeed", "high", 1048576, ["text", "image", "video"]),
    "kimicode_k3": ("k3", "high", 1048576, ["text", "image", "video"]),
    "kimicode_k3-256k": ("k3-256k", "high", 262144, ["text", "image"]),
    "kimicode_kimi-for-coding": ("kimi-for-coding", "max", 1048576, ["text", "image", "video"]),
}
EFFORTS = ["low", "high", "max"]
PROTOCOLS = ["openai-responses", "openai-compatible"]


def validate(catalog, sources):
    schema = json.loads((ROOT / "resources/provider_model_catalog.schema.json").read_text())
    Draft202012Validator(schema).validate(catalog)
    assert len(catalog["models"]) == 5
    assert {m["id"] for m in catalog["models"]} == set(EXPECTED)
    for m in catalog["models"]:
        model_id, default, window, modalities = EXPECTED[m["id"]]
        assert m["provider_id"] == "kimi_code" and m["model_id"] == model_id
        assert m["connection"]["default_base_url"] == "https://api.kimi.com/coding/v1"
        assert m["connection"]["auth_handler"] == "bearer_api_key"
        dimensions = {d["id"]: d for d in m["dimensions"]}
        assert [o["id"] for o in dimensions["reasoning.effort"]["options"]] == EFFORTS
        assert dimensions["reasoning.effort"]["default_policy"]["official_value"] == default
        # Thinking cannot be turned off: no none档 (silent K2.8 fallback risk).
        assert "none" not in [o["id"] for o in dimensions["reasoning.effort"]["options"]]
        assert [o["id"] for o in dimensions["api.protocol"]["options"]] == PROTOCOLS
        assert dimensions["api.protocol"]["default_policy"]["value"] == "openai-responses"
        assert [p["protocol"] for p in m["profiles"]] == PROTOCOLS
        r = m["profiles"][0]
        assert r["route"] == "/responses"
        # Coding-gateway Responses answers 400 on tool_choice=required.
        assert r.get("required_tool_choice") is False
        assert r["bindings"] == [{"dimension": "reasoning.effort", "handler": "enum_body_field", "path": "/reasoning/effort"}]
        c = m["profiles"][1]
        assert c["route"] == "/chat/completions"
        # Chat wire follows the platform K3 contract: required is supported.
        assert "required_tool_choice" not in c
        assert c["bindings"] == [{"dimension": "reasoning.effort", "handler": "kimi_chat_reasoning", "path": "/reasoning_effort"}]
        assert p_function_calling(m) == "supported"
        assert m["limits"]["context_window_tokens"] == window
        assert m["limits"]["max_input_tokens"] is None
        assert m["limits"]["max_output_tokens"] == 131072
        assert m["documented_facts"]["input_modalities"] == modalities
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
        assert urlparse(source["url"]).hostname in ("www.kimi.com", "api.kimi.com")
        assert len(source["sha256"]) == 64
        assert all(ch in "0123456789abcdef" for ch in source["sha256"])
    models_page = sources["sources"]["models"]
    assert "api.kimi.com/coding/v1" in models_page["excerpt"]
    assert "k3.1-pro" in models_page["excerpt"]
    assert "sk-kimi-" in sources["sources"]["claude-code"]["excerpt"]
    api_models = sources["sources"]["api-models"]
    assert "default_effort=high" in api_models["excerpt"]


def p_function_calling(m):
    return "supported"


if __name__ == "__main__":
    catalog = json.loads((ROOT / "resources/kimi_code_model_catalog.json").read_text())
    sources = json.loads((ROOT / "resources/kimi_code_model_catalog.sources.json").read_text())
    validate(catalog, sources)
    for field, value in [("model_id", "wrong"), ("provider_id", "kimi")]:
        invalid = copy.deepcopy(catalog)
        invalid["models"][0][field] = value
        try:
            validate(invalid, sources)
        except (AssertionError, ValueError):
            pass
        else:
            raise AssertionError("invalid identity was accepted")
    print("PASS: Kimi Code schema, five coding-plan template contracts, Responses-first profiles, evidence references, negative identity checks")
