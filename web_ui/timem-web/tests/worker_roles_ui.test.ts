import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { applyWorkerRoleMutation, replayWorkerRoleMutations } from "../src/worker_roles_ui";
import { WorkerRoleLibrary } from "../src/protocol";

const authoritative: WorkerRoleLibrary = {
  roles: [{ id: "role-existing", name: "Reviewer", description: "Review evidence." }],
  groups: [],
};

describe("optimistic worker role UI", () => {
  it("shows a client-identified create immediately without mutating authoritative state", () => {
    const visible = applyWorkerRoleMutation(authoritative, {
      type: "worker_role_create",
      session_id: "session-a",
      role_id: "role-client",
      name: " Builder ",
      description: " Build carefully. ",
    });

    expect(visible.roles).toEqual([
      authoritative.roles[0],
      { id: "role-client", name: "Builder", description: "Build carefully." },
    ]);
    expect(authoritative.roles).toHaveLength(1);
  });

  it("replays later pending edits over an authoritative acknowledgement", () => {
    const visible = replayWorkerRoleMutations(authoritative, [
      { type: "worker_role_update", session_id: "session-a", role_id: "role-existing", name: "Evidence reviewer", description: "Check logs." },
      { type: "worker_role_create", session_id: "session-a", role_id: "role-new", name: "Builder", description: "Build it." },
    ]);

    expect(visible.roles).toEqual([
      { id: "role-existing", name: "Evidence reviewer", description: "Check logs." },
      { id: "role-new", name: "Builder", description: "Build it." },
    ]);
  });

  it("can roll back a rejected mutation by replaying only remaining pending work", () => {
    const pending = new Map([
      ["rejected", { type: "worker_role_update", session_id: "session-a", role_id: "role-existing", name: "Rejected", description: "Rejected." } as const],
      ["remaining", { type: "worker_role_create", session_id: "session-a", role_id: "role-new", name: "Builder", description: "Build it." } as const],
    ]);
    pending.delete("rejected");

    expect(replayWorkerRoleMutations(authoritative, pending.values()).roles).toEqual([
      authoritative.roles[0],
      { id: "role-new", name: "Builder", description: "Build it." },
    ]);
  });
  it("wires optimistic create/edit reconciliation into the application event flow", () => {
    const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

    expect(source).toContain('clientId("worker-role-command")');
    expect(source).toContain('role_id: command.role_id ?? clientId("role")');
    expect(source).toContain("pendingWorkerRoleMutationsRef.current.delete(event.command_id)");
    expect(source).toContain("replayWorkerRoleMutations(event.library, pendingWorkerRoleMutationsRef.current.values())");
  });

});
