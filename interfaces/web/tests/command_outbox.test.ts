import { describe, expect, it } from "vitest";
import { commandNeedsReliableDelivery } from "../src/command_outbox";

describe("live command correlation", () => {
  it("correlates mutations without creating a browser outbox", () => {
    expect(
      commandNeedsReliableDelivery({
        type: "turn_submit",
        session_id: "s",
        text: "work",
      }),
    ).toBe(true);
    expect(
      commandNeedsReliableDelivery({ type: "session_delete", session_id: "s" }),
    ).toBe(true);
    expect(
      commandNeedsReliableDelivery({
        type: "model_endpoint_upsert",
        endpoint: {
          name: "prod",
          model: "gpt",
          api_protocol: "openai-compatible",
          response_protocol: "xml",
          base_url: "https://api.example",
          max_llm_input_tokens: 200_000,
          max_llm_output_tokens: 20_000,
          stream: true,
          api_key: "secret",
        },
      }),
    ).toBe(true);
  });

  it("leaves request-scoped reads uncorrelated and best effort", () => {
    expect(
      commandNeedsReliableDelivery({ type: "history_page", session_id: "s" }),
    ).toBe(false);
    expect(
      commandNeedsReliableDelivery({
        type: "model_endpoint_secret_reveal",
        endpoint_id: "one",
      }),
    ).toBe(false);
  });
});
