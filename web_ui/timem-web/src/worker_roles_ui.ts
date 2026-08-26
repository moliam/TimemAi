import { ClientCommand, WorkerRoleLibrary } from "./protocol";

export type WorkerRoleMutation = Extract<ClientCommand,
  | { type: "worker_role_create" }
  | { type: "worker_role_update" }
>;

export function isOptimisticWorkerRoleMutation(command: ClientCommand): command is WorkerRoleMutation {
  return command.type === "worker_role_create" || command.type === "worker_role_update";
}

export function applyWorkerRoleMutation(
  library: WorkerRoleLibrary,
  command: WorkerRoleMutation,
): WorkerRoleLibrary {
  if (command.type === "worker_role_create") {
    if (!command.role_id || library.roles.some((role) => role.id === command.role_id)) return library;
    return {
      ...library,
      roles: [...library.roles, {
        id: command.role_id,
        name: command.name.trim(),
        description: command.description.trim(),
      }],
    };
  }

  let changed = false;
  const roles = library.roles.map((role) => {
    if (role.id !== command.role_id) return role;
    changed = true;
    return {
      ...role,
      name: command.name.trim(),
      description: command.description.trim(),
    };
  });
  return changed ? { ...library, roles } : library;
}

export function replayWorkerRoleMutations(
  authoritative: WorkerRoleLibrary,
  pending: Iterable<WorkerRoleMutation>,
): WorkerRoleLibrary {
  let library = authoritative;
  for (const command of pending) library = applyWorkerRoleMutation(library, command);
  return library;
}
