# Cadence Roadmap

## Milestone 0 — Foundation
- [ ] Establish project documentation and structure
- [ ] GTK4/libadwaita application shell
- [ ] Build and test baseline
- [ ] Keep UI and core boundaries clear

## Milestone 1 — Device and repository discovery
- [ ] Detect storage devices and mount points
- [ ] Identify filesystem types
- [ ] Validate destination suitability
- [ ] Display information without modifying disks

## Milestone 2 — Backup planning
- [ ] Source selection and exclusions
- [ ] Destination/repository model
- [ ] Preflight checks
- [ ] Safe command planning and dry-run

## Milestone 3 — Backup execution
- [ ] RSYNC worker abstraction
- [ ] Asynchronous operation
- [ ] Progress and status reporting
- [ ] Error handling and cancellation

## Milestone 4 — Snapshot management
- [ ] Snapshot metadata
- [ ] Browse snapshots
- [ ] Integrity and accessibility checks

## Milestone 5 — Restore
- [ ] Restore planning and validation
- [ ] User confirmation and safety safeguards
- [ ] Restore execution and reporting

## Milestone 6 — Retention and advanced features
- [ ] Retention policy
- [ ] Safe cleanup
- [ ] Additional options only after core reliability is established

Milestones may be adjusted as implementation and testing reveal requirements.
