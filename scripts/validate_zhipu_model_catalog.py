"""Offline contract checks for the three implemented Zhipu Chat templates."""
import copy
import json
from pathlib import Path
from urllib.parse import urlparse
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
EXPECTED = {
    "z-glm5.2": ("glm-5.2", ["none", "high", "max"]),
    "z-glm5.3": ("glm-5.3", ["low", "high", "max"]),
    "z-glm5.3-flash": ("glm-5.3-flash", ["low", "high", "max"]),
}


def validate(catalog, sources):
    schema = json.loads((ROOT / "resources/provider_model_catalog.schema.json").read_text())
    Draft202012Validator(schema).validate(catalog)
    assert len(catalog["models"]) == 3
    assert {m["id"] for m in catalog["models"]} == set(EXPECTED)
    for m in catalog["models"]:
        model, efforts = EXPECTED[m["id"]]
        assert m["model_id"] == model and m["provider_id"] == "zhipu"
        assert m["connection"]["default_base_url"] == "https://open.bigmodel.cn/api/paas/v4"
        assert m["connection"]["auth_handler"] == "bearer_api_key"
        dimensions = {d["id"]: d for d in m["dimensions"]}
        assert [o["id"] for o in dimensions["reasoning.effort"]["options"]] == efforts
        assert dimensions["reasoning.effort"]["default_policy"]["official_value"] == "max"
        assert len(m["profiles"]) == 1
        p = m["profiles"][0]
        assert p["protocol"] == "openai-compatible" and p["route"] == "/chat/completions"
        assert p["bindings"] == [{"dimension": "reasoning.effort", "handler": "zhipu_chat_reasoning", "path": "/reasoning_effort"}]
        assert m["limits"]["context_window_tokens"] == 1000000
        assert m["limits"]["max_input_tokens"] is None
        assert m["limits"]["max_output_tokens"] == 131072

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
        assert urlparse(source["url"]).hostname == "docs.bigmodel.cn"
        assert len(source["sha256"]) == 64
        assert all(c in "0123456789abcdef" for c in source["sha256"])
    assert "131072" in sources["sources"]["chat-api"]["excerpt"]
    assert "reasoning_content" in sources["sources"]["thinking-mode"]["excerpt"]


if __name__ == "__main__":
    catalog = json.loads((ROOT / "resources/zhipu_model_catalog.json").read_text())
    sources = json.loads((ROOT / "resources/zhipu_model_catalog.sources.json").read_text())
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
    print("PASS: Zhipu schema, three template contracts, evidence references, negative identity checks")
