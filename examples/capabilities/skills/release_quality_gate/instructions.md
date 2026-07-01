# Release Quality Gate

Use this skill when preparing a Timem release or judging whether a release is
ready.

Checklist:

1. Confirm the working tree is clean or intentionally staged.
2. Run the relevant local tests and the production CI gate.
3. Run the sensitive information scan before publishing public artifacts.
4. Confirm the release tag points to the intended commit.
5. Review release notes for user-facing clarity and accurate scope.
6. If any feature touched terminal UI, include real TTY smoke evidence.

Do not claim release readiness without concrete command output or GitHub CI
evidence.
