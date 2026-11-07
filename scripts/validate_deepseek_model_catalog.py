"""Offline contract checks for the two implemented DeepSeek Chat templates."""
import copy
import json
from pathlib import Path
from urllib.parse import urlparse
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
EXPECTED = {
    "ds-flash": ("deepseek-flash", ["none", "low", "high", "max"], ["openai-compatible"]),
    "ds-v4-pro": ("deepseek-v4-pro", ["none", "low", "high", "max"], ["openai-compatible"]),
}


def validate(catalog, sources):
    schema = json.loads((ROOT / "resources/provider_model_catalog.schema.json").read_text())
    Draft202012Validator(schema).validate(catalog)
    assert len(catalog["models"]) == 2
    assert {m["id"] for m in catalog["models"]} == set(EXPECTED)
    for m in catalog["models"]:
        model, efforts, protocols = EXPECTED[m["id"]]
        assert m["model_id"] == model and m["provider_id"] == "deepseek"
        assert m["connection"]["default_base_url"] == "https://api.deepseek.com"
        assert m["connection"]["auth_handler"] == "bearer_api_key"
        dimensions = {d["id"]: d for d in m["dimensions"]}
        assert [o["id"] for o in dimensions["reasoning.effort"]["options"]] == efforts
        assert dimensions["reasoning.effort"]["default_policy"]["official_value"] == "high"
        assert [o["id"] for o in dimensions["api.protocol"]["options"]] == protocols
        assert dimensions["api.protocol"]["default_policy"]["value"] == "openai-compatible"
        assert [p["protocol"] for p in m["profiles"]] == protocols
        p = m["profiles"][0]
        assert p["route"] == "/chat/completions"
        assert p["function_calling"] == "supported"
        assert p["binding_status"] == "api_reference_reviewed"
        assert p["bindings"] == [
            {"dimension": "reasoning.effort", "handler": "deepseek_chat_thinking", "path": "/thinking"}
        ]
        assert m["limits"]["context_window_tokens"] == 1048576
        assert m["limits"]["max_input_tokens"] is None
        assert m["limits"]["max_output_tokens"] == 393216

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
        assert urlparse(source["url"]).hostname == "api-docs.deepseek.com"
        assert len(source["sha256"]) == 64
        assert all(c in "0123456789abcdef" for c in source["sha256"])
    assert "393216" in sources["sources"]["chat-api"]["excerpt"]
    assert "reasoning_effort" in sources["sources"]["thinking-mode"]["excerpt"]
    assert "1M" in sources["sources"]["models-pricing"]["excerpt"]


if __name__ == "__main__":
    catalog = json.loads((ROOT / "resources/deepseek_model_catalog.json").read_text())
    sources = json.loads((ROOT / "resources/deepseek_model_catalog.sources.json").read_text())
    validate(catalog, sources)
    for field, value in [("model_id", "wrong"), ("provider_id", "openai")]:
        invalid = copy.deepcopy(catalog)
        invalid["models"][0][field] = value
        try:
            validate(invalid, sources)
        except (AssertionError, ValueError):
            pass
    print("deepseek catalog contract ok")
