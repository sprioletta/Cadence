use crate::mount_inventory::{FilesystemClass, MountRecord};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserDataMode {
    All,
    HiddenOnly,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BackupScope {
    System,
    UserData,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePlanOptions {
    pub system_enabled: bool,
    pub user_data_enabled: bool,
    pub user_data_mode: UserDataMode,
    pub select_users_explicitly: bool,
    pub all_home_directories: Vec<PathBuf>,
    pub selected_home_directories: Vec<PathBuf>,
    pub system_custom_exclusions: Vec<PathBuf>,
    pub user_custom_exclusions: Vec<PathBuf>,
    pub destination_path: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedSource {
    pub mount_id: u64,
    pub path: PathBuf,
    pub scope: BackupScope,
    /// The source must not cross this mount boundary. Other eligible mounts
    /// are represented by their own PlannedSource entries.
    pub stay_on_filesystem: bool,
    pub exclusions: Vec<PathBuf>,
    pub user_data_mode: Option<UserDataMode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedMount {
    pub mount_id: u64,
    pub mount_point: PathBuf,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SourcePlan {
    pub sources: Vec<PlannedSource>,
    pub skipped_mounts: Vec<SkippedMount>,
    pub warnings: Vec<String>,
}

const SYSTEM_DEFAULT_EXCLUSIONS: &[&str] = &[
    "/dev",
    "/proc",
    "/sys",
    "/run",
    "/tmp",
    "/var/tmp",
    "/var/cache",
];

pub fn build_source_plan(
    mounts: &[MountRecord],
    options: &SourcePlanOptions,
) -> Result<SourcePlan, String> {
    validate_options(options)?;

    let root_mount = mounts
        .iter()
        .filter(|mount| mount.mount_point == Path::new("/"))
        .max_by_key(|mount| mount.mount_id)
        .ok_or_else(|| "The mount inventory does not contain the system root (/).".to_string())?;

    let user_data_active =
        options.user_data_enabled && options.user_data_mode != UserDataMode::None;
    let all_homes = normalized_paths(&options.all_home_directories);
    let selected_homes = selected_homes(options);

    let mut plan = SourcePlan::default();
    let mut eligible_mounts: Vec<&MountRecord> = Vec::new();

    for mount in mounts {
        let reason = skipped_mount_reason(mount);
        if let Some(reason) = reason {
            plan.skipped_mounts.push(SkippedMount {
                mount_id: mount.mount_id,
                mount_point: mount.mount_point.clone(),
                reason,
            });
        } else {
            eligible_mounts.push(mount);
        }
    }

    eligible_mounts.sort_by(|left, right| {
        left.mount_point
            .components()
            .count()
            .cmp(&right.mount_point.components().count())
            .then_with(|| left.mount_point.cmp(&right.mount_point))
            .then_with(|| left.mount_id.cmp(&right.mount_id))
    });

    for mount in &eligible_mounts {
        let non_root_filesystem_root = mount.root != Path::new("/");
        let same_device_mounted_elsewhere = eligible_mounts.iter().any(|other| {
            other.mount_id != mount.mount_id
                && other.major_minor == mount.major_minor
                && other.mount_point != mount.mount_point
        });

        if non_root_filesystem_root || same_device_mounted_elsewhere {
            plan.warnings.push(format!(
                "Mount {} may be a bind mount or filesystem subvolume; its relationship to other sources requires review.",
                mount.mount_point.display()
            ));
        }
    }

    if options.system_enabled {
        for mount in &eligible_mounts {
            let mount_point = mount.mount_point.as_path();

            // A filesystem mounted inside a home directory belongs to the
            // user-data scope, not the system scope.
            if user_data_active && all_homes.iter().any(|home| mount_point.starts_with(home)) {
                continue;
            }

            let homes_to_exclude = if user_data_active {
                all_homes.as_slice()
            } else {
                &[]
            };
            let mut exclusions =
                system_exclusions_for_mount(options, mount_point, homes_to_exclude);
            exclusions.sort();
            exclusions.dedup();

            // If an exclusion is this mount point or one of its ancestors,
            // this filesystem must not become a source of its own.
            if exclusions
                .iter()
                .any(|excluded| mount_point.starts_with(excluded))
            {
                continue;
            }

            plan.sources.push(PlannedSource {
                mount_id: mount.mount_id,
                path: mount.mount_point.clone(),
                scope: BackupScope::System,
                stay_on_filesystem: true,
                exclusions,
                user_data_mode: None,
            });
        }
    }

    if user_data_active {
        for home in &selected_homes {
            let Some(mount) = deepest_mount_for_path(&eligible_mounts, home) else {
                plan.warnings.push(format!(
                    "No eligible mounted filesystem contains user home {}.",
                    home.display()
                ));
                continue;
            };

            let blocked_by_skipped_mount = plan.skipped_mounts.iter().any(|skipped| {
                home.starts_with(&skipped.mount_point)
                    && skipped.mount_point != mount.mount_point
                    && skipped.mount_point.starts_with(&mount.mount_point)
            });
            if blocked_by_skipped_mount {
                plan.warnings.push(format!(
                    "User home {} is located on or below an excluded filesystem mount.",
                    home.display()
                ));
                continue;
            }

            if user_source_is_excluded(options, home) {
                plan.warnings.push(format!(
                    "User home {} is covered by a user-data exclusion.",
                    home.display()
                ));
                continue;
            }

            let exclusions = user_exclusions_for_path(options, home);
            plan.sources.push(PlannedSource {
                mount_id: mount.mount_id,
                path: home.clone(),
                scope: BackupScope::UserData,
                stay_on_filesystem: true,
                exclusions,
                user_data_mode: Some(options.user_data_mode),
            });
        }

        // Include eligible filesystems mounted below a selected home as
        // independent user-data sources. The home source itself stays on its
        // own filesystem, so the nested mount is never traversed implicitly.
        for mount in &eligible_mounts {
            let mount_point = mount.mount_point.as_path();
            if selected_homes
                .iter()
                .any(|home| mount_point != home && mount_point.starts_with(home))
                && !user_source_is_excluded(options, mount_point)
            {
                if !plan.sources.iter().any(|source| {
                    source.scope == BackupScope::UserData && source.path == mount.mount_point
                }) {
                    plan.sources.push(PlannedSource {
                        mount_id: mount.mount_id,
                        path: mount.mount_point.clone(),
                        scope: BackupScope::UserData,
                        stay_on_filesystem: true,
                        exclusions: user_exclusions_for_path(options, mount_point),
                        user_data_mode: Some(options.user_data_mode),
                    });
                }
            }
        }
    }

    // The root mount is essential for a system plan. Do not silently return a
    // partial system plan if the inventory contains only unusable root data.
    if options.system_enabled
        && !plan.sources.iter().any(|source| {
            source.scope == BackupScope::System && source.mount_id == root_mount.mount_id
        })
    {
        return Err(
            "The system root filesystem cannot be included by the current mount policy."
                .to_string(),
        );
    }

    plan.sources.sort_by(|left, right| {
        left.scope
            .cmp(&right.scope)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.mount_id.cmp(&right.mount_id))
    });

    Ok(plan)
}

fn validate_options(options: &SourcePlanOptions) -> Result<(), String> {
    for path in options
        .all_home_directories
        .iter()
        .chain(options.selected_home_directories.iter())
        .chain(options.system_custom_exclusions.iter())
        .chain(options.user_custom_exclusions.iter())
        .chain(options.destination_path.iter())
    {
        validate_absolute_path(path)?;
    }

    if options
        .selected_home_directories
        .iter()
        .any(|selected| !options.all_home_directories.contains(selected))
    {
        return Err(
            "Explicitly selected user homes must come from the detected local-user list."
                .to_string(),
        );
    }

    Ok(())
}

fn validate_absolute_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path == Path::new("/")
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!(
            "Path must be absolute and cannot contain '..': {}",
            path.display()
        ));
    }

    Ok(())
}

fn selected_homes(options: &SourcePlanOptions) -> Vec<PathBuf> {
    if options.select_users_explicitly {
        normalized_paths(&options.selected_home_directories)
    } else {
        normalized_paths(&options.all_home_directories)
    }
}

fn normalized_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut unique = paths.to_vec();
    unique.sort();
    unique.dedup();
    unique
}

fn skipped_mount_reason(mount: &MountRecord) -> Option<String> {
    if !mount.mount_point.is_absolute() {
        return Some("Mount point is not an absolute path.".to_string());
    }

    match classify_filesystem_name(&mount.filesystem) {
        FilesystemClass::Virtual => {
            return Some(format!(
                "Virtual filesystem ({}) is not a backup source.",
                mount.filesystem
            ));
        }
        FilesystemClass::Network => {
            return Some(format!(
                "Network filesystem ({}) is excluded by default.",
                mount.filesystem
            ));
        }
        FilesystemClass::Unknown => {
            return Some(format!(
                "Unknown filesystem ({}) requires an explicit policy.",
                mount.filesystem
            ));
        }
        FilesystemClass::Local => {}
    }

    if is_external_mount(&mount.mount_point) {
        return Some(
            "External mount under /mnt or /media is not included automatically.".to_string(),
        );
    }

    None
}

fn classify_filesystem_name(filesystem: &str) -> FilesystemClass {
    match filesystem {
        "proc" | "sysfs" | "devtmpfs" | "devpts" | "tmpfs" | "cgroup" | "cgroup2" | "pstore"
        | "securityfs" | "debugfs" | "tracefs" | "configfs" | "mqueue" | "hugetlbfs"
        | "rpc_pipefs" | "autofs" | "bpf" | "efivarfs" | "fusectl" => FilesystemClass::Virtual,
        "nfs" | "nfs4" | "cifs" | "smb3" | "9p" | "ceph" | "glusterfs" | "afs" | "davfs"
        | "sshfs" => FilesystemClass::Network,
        "ext2" | "ext3" | "ext4" | "xfs" | "btrfs" | "vfat" | "fat" | "msdos" | "exfat"
        | "ntfs" | "ntfs3" | "f2fs" | "zfs" | "reiserfs" | "jfs" | "bcachefs" | "iso9660" => {
            FilesystemClass::Local
        }
        _ => FilesystemClass::Unknown,
    }
}

fn is_external_mount(path: &Path) -> bool {
    ["/mnt", "/media"].iter().any(|base| {
        let base = Path::new(base);
        path == base || path.starts_with(base)
    })
}

fn deepest_mount_for_path<'a>(mounts: &[&'a MountRecord], path: &Path) -> Option<&'a MountRecord> {
    mounts
        .iter()
        .copied()
        .filter(|mount| path.starts_with(&mount.mount_point))
        .max_by_key(|mount| (mount.mount_point.components().count(), mount.mount_id))
}

fn system_exclusions_for_mount(
    options: &SourcePlanOptions,
    mount_point: &Path,
    selected_homes: &[PathBuf],
) -> Vec<PathBuf> {
    let mut exclusions = Vec::new();

    for default in SYSTEM_DEFAULT_EXCLUSIONS {
        let path = Path::new(default);
        if path.starts_with(mount_point) || mount_point.starts_with(path) {
            exclusions.push(path.to_path_buf());
        }
    }

    for home in selected_homes {
        if home.starts_with(mount_point) {
            exclusions.push(home.clone());
        }
    }

    for path in &options.system_custom_exclusions {
        if path.starts_with(mount_point) || mount_point.starts_with(path) {
            exclusions.push(path.clone());
        }
    }

    if let Some(destination) = &options.destination_path {
        if destination.starts_with(mount_point) || mount_point.starts_with(destination) {
            exclusions.push(destination.clone());
        }
    }

    exclusions
}

fn user_source_is_excluded(options: &SourcePlanOptions, source_path: &Path) -> bool {
    options
        .user_custom_exclusions
        .iter()
        .any(|excluded| source_path.starts_with(excluded))
        || options
            .destination_path
            .as_ref()
            .is_some_and(|destination| source_path.starts_with(destination))
}

fn user_exclusions_for_path(options: &SourcePlanOptions, source_path: &Path) -> Vec<PathBuf> {
    let mut exclusions: Vec<PathBuf> = options
        .user_custom_exclusions
        .iter()
        .filter(|path| path.starts_with(source_path))
        .cloned()
        .collect();

    if let Some(destination) = &options.destination_path {
        if destination.starts_with(source_path) {
            exclusions.push(destination.clone());
        }
    }

    exclusions.sort();
    exclusions.dedup();
    exclusions
}

#[cfg(test)]
mod tests {
    use super::{BackupScope, SourcePlanOptions, UserDataMode, build_source_plan};
    use crate::mount_inventory::MountRecord;
    use std::path::PathBuf;

    fn mount(id: u64, point: &str, filesystem: &str) -> MountRecord {
        MountRecord {
            mount_id: id,
            parent_id: id.saturating_sub(1),
            major_minor: "8:1".to_string(),
            root: PathBuf::from("/"),
            mount_point: PathBuf::from(point),
            mount_options: vec!["rw".to_string()],
            optional_fields: Vec::new(),
            filesystem: filesystem.to_string(),
            source: format!("/dev/test{id}"),
            super_options: vec!["rw".to_string()],
        }
    }

    fn options() -> SourcePlanOptions {
        SourcePlanOptions {
            system_enabled: true,
            user_data_enabled: true,
            user_data_mode: UserDataMode::All,
            select_users_explicitly: false,
            all_home_directories: vec![PathBuf::from("/home/alice")],
            selected_home_directories: Vec::new(),
            system_custom_exclusions: Vec::new(),
            user_custom_exclusions: Vec::new(),
            destination_path: None,
        }
    }

    fn source_paths(plan: &super::SourcePlan, scope: BackupScope) -> Vec<PathBuf> {
        plan.sources
            .iter()
            .filter(|source| source.scope == scope)
            .map(|source| source.path.clone())
            .collect()
    }

    #[test]
    fn includes_root_and_separate_local_system_mounts() {
        let mounts = vec![
            mount(1, "/", "ext4"),
            mount(2, "/usr", "xfs"),
            mount(3, "/var", "ext4"),
            mount(4, "/opt", "btrfs"),
            mount(5, "/srv", "ext4"),
        ];

        let plan = build_source_plan(&mounts, &options()).unwrap();
        assert_eq!(
            source_paths(&plan, BackupScope::System),
            vec![
                PathBuf::from("/"),
                PathBuf::from("/opt"),
                PathBuf::from("/usr"),
                PathBuf::from("/srv"),
                PathBuf::from("/var")
            ]
        );
    }

    #[test]
    fn excludes_selected_home_from_system_and_adds_it_to_user_scope() {
        let plan = build_source_plan(&[mount(1, "/", "ext4")], &options()).unwrap();

        let system = plan
            .sources
            .iter()
            .find(|source| source.scope == BackupScope::System)
            .unwrap();
        assert!(system.exclusions.contains(&PathBuf::from("/home/alice")));
        assert_eq!(
            source_paths(&plan, BackupScope::UserData),
            vec![PathBuf::from("/home/alice")]
        );
    }

    #[test]
    fn includes_home_in_system_when_user_data_is_disabled() {
        let mut config = options();
        config.user_data_enabled = false;

        let plan = build_source_plan(&[mount(1, "/", "ext4")], &config).unwrap();
        let system = plan.sources.iter().find(|source| source.scope == BackupScope::System).unwrap();

        assert!(!system.exclusions.contains(&PathBuf::from("/home/alice")));
        assert!(source_paths(&plan, BackupScope::UserData).is_empty());
    }

    #[test]
    fn includes_home_in_system_when_user_data_mode_is_none() {
        let mut config = options();
        config.user_data_mode = UserDataMode::None;

        let plan = build_source_plan(&[mount(1, "/", "ext4")], &config).unwrap();
        let system = plan.sources.iter().find(|source| source.scope == BackupScope::System).unwrap();

        assert!(!system.exclusions.contains(&PathBuf::from("/home/alice")));
        assert!(source_paths(&plan, BackupScope::UserData).is_empty());
    }

    #[test]
    fn selected_user_mode_excludes_all_homes_but_backs_up_only_selected_accounts() {
        let mut config = options();
        config.select_users_explicitly = true;
        config.all_home_directories =
            vec![PathBuf::from("/home/alice"), PathBuf::from("/home/bob")];
        config.selected_home_directories = vec![PathBuf::from("/home/alice")];

        let plan = build_source_plan(&[mount(1, "/", "ext4")], &config).unwrap();
        let system = plan.sources.iter().find(|source| source.scope == BackupScope::System).unwrap();

        assert!(system.exclusions.contains(&PathBuf::from("/home/alice")));
        assert!(system.exclusions.contains(&PathBuf::from("/home/bob")));
        assert_eq!(
            source_paths(&plan, BackupScope::UserData),
            vec![PathBuf::from("/home/alice")]
        );
    }

    #[test]
    fn keeps_parent_volume_for_non_home_data_and_excludes_nested_home() {
        let mut config = options();
        config.all_home_directories = vec![PathBuf::from("/data/users/alice")];

        let plan =
            build_source_plan(&[mount(1, "/", "ext4"), mount(2, "/data", "xfs")], &config).unwrap();

        let data = plan
            .sources
            .iter()
            .find(|source| {
                source.scope == BackupScope::System && source.path == PathBuf::from("/data")
            })
            .unwrap();

        assert!(
            data.exclusions
                .contains(&PathBuf::from("/data/users/alice"))
        );
        assert!(plan.sources.iter().any(|source| {
            source.scope == BackupScope::UserData
                && source.path == PathBuf::from("/data/users/alice")
                && source.mount_id == 2
        }));
    }

    #[test]
    fn flags_non_root_filesystem_roots_and_repeated_device_mounts_for_review() {
        let plan = build_source_plan(
            &[mount(1, "/", "ext4"), mount(2, "/var/lib/data", "ext4")],
            &options(),
        )
        .unwrap();

        assert!(plan.warnings.iter().any(|warning| {
            warning.contains("/var/lib/data")
                && warning.contains("bind mount or filesystem subvolume")
        }));
    }

    #[test]
    fn includes_separate_boot_filesystems_as_system_sources() {
        let plan = build_source_plan(
            &[
                mount(1, "/", "ext4"),
                mount(2, "/boot", "ext4"),
                mount(3, "/boot/efi", "vfat"),
            ],
            &options(),
        )
        .unwrap();

        assert!(source_paths(&plan, BackupScope::System).contains(&PathBuf::from("/boot")));
        assert!(source_paths(&plan, BackupScope::System).contains(&PathBuf::from("/boot/efi")));
    }

    #[test]
    fn skips_virtual_network_unknown_and_external_mounts() {
        let plan = build_source_plan(
            &[
                mount(1, "/", "ext4"),
                mount(2, "/proc", "proc"),
                mount(3, "/network", "nfs4"),
                mount(4, "/mystery", "fuse.custom"),
                mount(5, "/mnt/disk", "ext4"),
                mount(6, "/media/disk", "xfs"),
            ],
            &options(),
        )
        .unwrap();

        assert_eq!(plan.skipped_mounts.len(), 5);
        assert_eq!(
            source_paths(&plan, BackupScope::System),
            vec![PathBuf::from("/")]
        );
    }

    #[test]
    fn handles_a_separate_home_filesystem_without_system_user_data_duplication() {
        let plan = build_source_plan(
            &[mount(1, "/", "ext4"), mount(2, "/home", "xfs")],
            &options(),
        )
        .unwrap();

        let home_system_mount = plan
            .sources
            .iter()
            .find(|source| {
                source.scope == BackupScope::System && source.path == PathBuf::from("/home")
            })
            .unwrap();
        assert!(
            home_system_mount
                .exclusions
                .contains(&PathBuf::from("/home/alice"))
        );
        assert!(plan.sources.iter().any(|source| {
            source.scope == BackupScope::UserData
                && source.path == PathBuf::from("/home/alice")
                && source.mount_id == 2
        }));
    }

    #[test]
    fn includes_separate_filesystem_nested_inside_home_only_as_user_data() {
        let plan = build_source_plan(
            &[
                mount(1, "/", "ext4"),
                mount(2, "/home/alice/archive", "xfs"),
            ],
            &options(),
        )
        .unwrap();

        assert!(
            !source_paths(&plan, BackupScope::System)
                .contains(&PathBuf::from("/home/alice/archive"))
        );
        assert!(
            source_paths(&plan, BackupScope::UserData)
                .contains(&PathBuf::from("/home/alice/archive"))
        );
    }

    #[test]
    fn applies_system_and_user_custom_exclusions_independently() {
        let mut config = options();
        config.system_custom_exclusions = vec![PathBuf::from("/var/log/private")];
        config.user_custom_exclusions = vec![PathBuf::from("/home/alice/Downloads")];

        let plan = build_source_plan(&[mount(1, "/", "ext4")], &config).unwrap();
        let system = plan
            .sources
            .iter()
            .find(|source| source.scope == BackupScope::System)
            .unwrap();
        let user = plan
            .sources
            .iter()
            .find(|source| source.scope == BackupScope::UserData)
            .unwrap();

        assert!(
            system
                .exclusions
                .contains(&PathBuf::from("/var/log/private"))
        );
        assert!(
            !system
                .exclusions
                .contains(&PathBuf::from("/home/alice/Downloads"))
        );
        assert!(
            user.exclusions
                .contains(&PathBuf::from("/home/alice/Downloads"))
        );
        assert!(!user.exclusions.contains(&PathBuf::from("/var/log/private")));
    }

    #[test]
    fn excludes_destination_from_any_source_that_contains_it() {
        let mut config = options();
        config.destination_path = Some(PathBuf::from("/mnt/backup/cadence-snap"));

        let plan = build_source_plan(
            &[mount(1, "/", "ext4"), mount(2, "/mnt/backup", "ext4")],
            &config,
        )
        .unwrap();

        assert!(plan.skipped_mounts.iter().any(|mount| mount.mount_id == 2));
        let root = plan
            .sources
            .iter()
            .find(|source| source.scope == BackupScope::System)
            .unwrap();
        assert!(
            root.exclusions
                .contains(&PathBuf::from("/mnt/backup/cadence-snap"))
        );
    }

    #[test]
    fn system_only_and_user_only_modes_produce_independent_plans() {
        let mounts = &[mount(1, "/", "ext4")];

        let mut system_only = options();
        system_only.user_data_enabled = false;
        let system_plan = build_source_plan(mounts, &system_only).unwrap();
        assert!(!system_plan.sources.is_empty());
        assert!(
            system_plan
                .sources
                .iter()
                .all(|source| source.scope == BackupScope::System)
        );

        let mut users_only = options();
        users_only.system_enabled = false;
        let user_plan = build_source_plan(mounts, &users_only).unwrap();
        assert!(!user_plan.sources.is_empty());
        assert!(
            user_plan
                .sources
                .iter()
                .all(|source| source.scope == BackupScope::UserData)
        );
    }

    #[test]
    fn does_not_fall_back_to_parent_filesystem_for_home_on_excluded_mount() {
        let plan = build_source_plan(
            &[mount(1, "/", "ext4"), mount(2, "/home", "nfs4")],
            &options(),
        )
        .unwrap();

        assert!(source_paths(&plan, BackupScope::UserData).is_empty());
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("excluded filesystem"))
        );
    }

    #[test]
    fn rejects_user_home_paths_not_present_in_the_detected_user_list() {
        let mut config = options();
        config.select_users_explicitly = true;
        config.selected_home_directories = vec![PathBuf::from("/etc")];

        assert!(build_source_plan(&[mount(1, "/", "ext4")], &config).is_err());
    }

    #[test]
    fn rejects_root_as_a_custom_exclusion_path() {
        let mut config = options();
        config.system_custom_exclusions = vec![PathBuf::from("/")];

        assert!(build_source_plan(&[mount(1, "/", "ext4")], &config).is_err());
    }

    #[test]
    fn rejects_relative_and_parent_traversal_paths() {
        let mut config = options();
        config.user_custom_exclusions = vec![PathBuf::from("../private")];

        assert!(build_source_plan(&[mount(1, "/", "ext4")], &config).is_err());
    }

    #[test]
    fn does_not_create_a_source_for_a_mountpoint_inside_a_system_exclusion() {
        let plan = build_source_plan(
            &[mount(1, "/", "ext4"), mount(2, "/var/cache", "xfs")],
            &options(),
        )
        .unwrap();

        assert!(!source_paths(&plan, BackupScope::System).contains(&PathBuf::from("/var/cache")));
    }

    #[test]
    fn does_not_create_a_user_source_covered_by_a_parent_exclusion() {
        let mut config = options();
        config.user_custom_exclusions = vec![PathBuf::from("/home")];

        let plan = build_source_plan(&[mount(1, "/", "ext4")], &config).unwrap();

        assert!(source_paths(&plan, BackupScope::UserData).is_empty());
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("covered by a user-data exclusion"))
        );
    }

    #[test]
    fn reports_missing_root_mount_instead_of_creating_partial_plan() {
        assert!(build_source_plan(&[mount(2, "/data", "ext4")], &options()).is_err());
    }
}
