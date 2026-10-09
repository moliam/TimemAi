# `context_compress` keep-semantics evaluation

## Scope

This report evaluates the prototype that changes `context_compress` from a
model-selected removal list to a model-selected retention whitelist:

- `summary` is required and becomes the authoritative compressed state.
- `keep` is optional. Only listed live delta IDs retain their original prompt
  content; every other live delta is removed.
- Omitting `keep` intentionally replaces all live deltas with `summary`.
- `offload` optionally writes important removed deltas to scratch before
  removal.
- Missing `keep`/`offload` references, `keep`/`offload` overlap, and legacy
  `discard` input fail closed.

The evaluation separates deterministic runtime behavior from exploratory model
behavior. A single deterministic regression case is not reported as a model
success rate.

## Deterministic historical-case A/B

A reconstructed historical weak-compression case was run through the real
`AgentCore::apply_model_response` path. The old side reconstructs the previous
selective-discard behavior; the new side uses summary-only replacement.

| Variant | Stale markers left | Dynamic deltas left | Estimated dynamic tokens |
|---|---:|---:|---:|
| Old selective discard | 5 | 6 | about 65 |
| New summary-only replacement | 0 | 1 | about 36 |

Regression test:

`prompt_component_tests::historical_wording_change_case_keep_semantics_eliminates_stale_raw_history`

This establishes the structural effect for that case only: omitted stale IDs no
longer survive by default. It does not estimate model-level reliability.

## Exploratory first-response model evaluation

### Design

- Model: `gpt-5.6-sol` through the locally configured model service.
- The new-contract arm used the final action-oriented prompt: inspect the live
  context, extract authoritative state, choose `keep`/`offload` sparingly, and
  self-check before calling. It did not explain Runtime deletion, telemetry,
  prompt-rewrite, or reinjection internals.
- Five reconstructed historical compression scenarios:
  `wording_change`, `attach_directory_gate`, `reasoning_dispatch`,
  `keep_contract_prototype`, and `running_job_runtime_authority`.
- Five independent first responses per scenario and contract.
- Total: 25 old-contract calls and 25 new-contract calls.
- The comparison used the same model and evaluation cases for both contracts.

A response counts as successful only when all three gates pass:

1. **Protocol:** executable under the contract being evaluated.
2. **Structure:** no annotated harmful raw delta survives compaction.
3. **Semantic fidelity:** every annotated fact group remains available either
   in `summary` or, for the new contract, in an explicitly retained `keep`
   delta.

The fair rescoring follows actual runtime semantics rather than the initial
pilot scorer:

- Historical `discard`/`offload` overlap is accepted and their union is removed,
  matching the former runtime.
- New `keep`/`offload` overlap fails closed.
- Facts preserved in an explicitly retained delta satisfy semantic fidelity;
  they need not be duplicated in `summary`.

### Results

| Contract | Protocol | Structure | Semantic fidelity | Overall first-response success | Wilson 95% CI |
|---|---:|---:|---:|---:|---:|
| Old selective discard | 25/25 | 25/25 | 17/25 | 17/25 (68%) | 48.4%–82.8% |
| New keep whitelist with final action-oriented prompt | 25/25 | 25/25 | 20/25 | 20/25 (80%) | 60.9%–91.1% |

Observed difference: **+12 percentage points** in this sample. This compares the
complete alternatives actually tested—old selective-discard semantics with the
old guidance versus keep-whitelist semantics with the final action-oriented
guidance. Because both semantics and instructions changed, the experiment does
not identify how much of the difference comes from either factor alone.

An earlier result of 24/25 for the new arm is retired: its fifth scenario
encoded an invalid premise that MCP schemas were persistent prompt deltas needing
post-compression reinjection. The corrected scenario tests only the independent
Runtime authority of still-running job state.

Per-scenario overall successes:

| Scenario | Old | New |
|---|---:|---:|
| `wording_change` | 4/5 | 2/5 |
| `attach_directory_gate` | 4/5 | 5/5 |
| `reasoning_dispatch` | 5/5 | 5/5 |
| `keep_contract_prototype` | 1/5 | 3/5 |
| `running_job_runtime_authority` | 3/5 | 5/5 |

Both contracts passed protocol and structural cleanup in every call. The
observed difference came from semantic fidelity. The new arm improved three
scenarios, tied one, and regressed on `wording_change`; the small per-scenario
sample is too limited to infer stable scenario-specific effects.

## Interpretation and limits

This is an exploratory, small-sample comparison, not a conclusive population
estimate. The Wilson intervals overlap, calls are clustered within five cases,
and all samples use one model/service configuration. The observed improvement
supports the keep-whitelist design and justifies broader evaluation, but it must
not be described as statistically proven superiority.

The initial pilot and the earlier keep-prompt run are deliberately not used
for the final table. The pilot scorer incorrectly rejected behavior accepted by
the historical runtime and failed to credit facts preserved in retained deltas;
the earlier run did not use the final action-oriented prompt. The table above
uses the corrected contract semantics and the final prompt.

Recommended follow-up:

- Increase the number and diversity of historical cases and model families.
- Pre-register semantic fact groups and scoring rules before sampling.
- Add blinded human review for summaries whose facts are semantically present
  but lexically different.
- Track token reduction and downstream task completion, not only the first
  compression response.

## Reproducibility evidence

Local evaluation artifacts were retained outside the repository to avoid
checking model outputs and endpoint metadata into source control:

- Corrected final-prompt results:
  `/tmp/context-compress-first-pass-corrected.json`
- Evaluation script:
  `/tmp/context_compress_first_pass_eval.py`

SHA-256:

- Corrected final-prompt results:
  `b760b7a8726ce3e34ecfad34ce191cca6b29ba4d22ae62881d29633db89d7730`
- Evaluation script:
  `686355b51e3c81b35d9ab15aece161a26324ee17ff8e82f4fa0ff9e3a55feaf1`
