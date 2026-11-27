# Cadence — Initial Architecture

## Principles

- Reliability and data integrity take priority over feature count.
- Keep the UI responsive during all long-running operations.
- Separate application UI, core logic, and backup execution.
- Avoid destructive operations unless they are validated and explicitly authorized.
- Develop in small milestones, each independently buildable and testable.

## Initial technology choices

- Rust
- GTK4 and libadwaita for the desktop UI
- RSYNC invoked as a subprocess for backup operations
- TOML for configuration (when configuration is introduced)
- PolicyKit for narrowly scoped privileged operations, when required

## Architectural boundaries

### UI layer
Owns widgets, user interaction, presentation, and UI state. It must not directly implement backup filesystem logic or block the GTK main thread.

### Core layer
Owns application concepts, validation, configuration, and policies. It should not depend on GTK.

### Backup execution layer
Owns command construction, subprocess execution, output parsing, cancellation and operation reporting. It should be isolated from widget code and testable independently.

### Privilege handling
Privileged access should be narrowly scoped and requested only when needed. Do not run the entire UI as root.

## Snapshot repository concepts

- Snapshot storage resides on a destination filesystem.
- Snapshot content should be treated as immutable after successful creation.
- RSYNC and hard-link snapshot techniques may be used where appropriate.
- Validate source, destination, filesystem, permissions and available space before execution.
- Retention and deletion are deferred until safeguards are designed and tested.

## Safety requirements

- Never silently overwrite existing snapshot data.
- Never delete snapshots during the initial milestones.
- Validate paths and mount state before operations.
- Provide clear errors and operation logs.
- Build a dry-run or validation strategy before enabling real backups.
- Cancellation and failure must not leave the UI in an ambiguous state.

## First milestone

Create and validate the GTK4/libadwaita application shell only. No backup execution, privilege escalation, repository manipulation, or filesystem mutation in this milestone.
