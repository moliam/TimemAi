import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// Serial: the actual-product harness owns a fixed loopback port and cleans up
// its temporary Host, fake model, Chrome, and MEM before the next scenario.
function run(file, env = {}) {
  const result = spawnSync(process.execPath, [fileURLToPath(new URL(file, import.meta.url))], {
    env: { ...process.env, ...env },
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
run("stream-preview-acceptance.mjs");
for (const protocol of ["xml", "json", "native"]) {
  run("stream-preview-product.mjs", {
    STREAM_PREVIEW_PROTOCOL: protocol,
    STREAM_PREVIEW_SCENARIO: "normal",
  });
}
for (const scenario of ["invalid", "network", "stop", "supplement", "interaction", "tools"]) {
  run("stream-preview-product.mjs", {
    STREAM_PREVIEW_PROTOCOL: "xml",
    STREAM_PREVIEW_SCENARIO: scenario,
  });
}
