use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountRecord {
    pub mount_id: u64,
    pub parent_id: u64,
    pub major_minor: String,
    pub root: PathBuf,
    pub mount_point: PathBuf,
    pub mount_options: Vec<String>,
    pub optional_fields: Vec<String>,
    pub filesystem: String,
    pub source: String,
    pub super_options: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilesystemClass {
    Virtual,
    Network,
    Local,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountRole {
    SystemRoot,
    Boot,
    UserData,
    External,
    SystemPathCandidate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountClassification {
    pub filesystem_class: FilesystemClass,
    pub role: MountRole,
    /// A non-root filesystem root can indicate a bind mount or a subvolume.
    /// mountinfo alone is not sufficient to distinguish those cases safely.
    pub has_non_root_filesystem_root: bool,
}

pub fn discover_mounts() -> Result<Vec<MountRecord>, String> {
    let contents = fs::read_to_string("/proc/self/mountinfo")
        .map_err(|error| format!("Could not read /proc/self/mountinfo: {error}"))?;
    parse_mountinfo(&contents)
}

pub fn parse_mountinfo(contents: &str) -> Result<Vec<MountRecord>, String> {
    let mut mounts = Vec::new();

    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }

        mounts.push(parse_mountinfo_line(line).map_err(|error| {
            format!("Invalid mountinfo entry on line {}: {error}", index + 1)
        })?);
    }

    Ok(mounts)
}

fn parse_mountinfo_line(line: &str) -> Result<MountRecord, String> {
    let (before_separator, after_separator) = line
        .split_once(" - ")
        .ok_or_else(|| "missing field separator".to_string())?;
    let before: Vec<&str> = before_separator.split_whitespace().collect();
    let after: Vec<&str> = after_separator.split_whitespace().collect();

    if before.len() < 6 || after.len() < 3 {
        return Err("not enough fields".to_string());
    }

    let mount_id = before[0]
        .parse::<u64>()
        .map_err(|_| "invalid mount ID".to_string())?;
    let parent_id = before[1]
        .parse::<u64>()
        .map_err(|_| "invalid parent mount ID".to_string())?;

    Ok(MountRecord {
        mount_id,
        parent_id,
        major_minor: before[2].to_string(),
        root: PathBuf::from(decode_mount_field(before[3])),
        mount_point: PathBuf::from(decode_mount_field(before[4])),
        mount_options: split_options(before[5]),
        optional_fields: before[6..].iter().map(|field| (*field).to_string()).collect(),
        filesystem: after[0].to_string(),
        source: decode_mount_field(after[1]),
        super_options: split_options(after[2]),
    })
}

fn split_options(value: &str) -> Vec<String> {
    value.split(',').map(str::to_string).collect()
}

fn decode_mount_field(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 3 < bytes.len()
            && bytes[index + 1..index + 4]
                .iter()
                .all(|byte| (b'0'..=b'7').contains(byte))
        {
            let octal = (bytes[index + 1] - b'0') * 64
                + (bytes[index + 2] - b'0') * 8
                + (bytes[index + 3] - b'0');
            decoded.push(octal);
            index += 4;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

pub fn classify_mount(
    mount: &MountRecord,
    user_home_directories: &[PathBuf],
) -> MountClassification {
    let filesystem_class = classify_filesystem(&mount.filesystem);
    let mount_point = mount.mount_point.as_path();

    let role = if mount_point == Path::new("/") {
        MountRole::SystemRoot
    } else if is_boot_path(mount_point) {
        MountRole::Boot
    } else if user_home_directories.iter().any(|home| {
        mount_point.starts_with(home) || home.starts_with(mount_point)
    }) {
        MountRole::UserData
    } else if is_external_path(mount_point) {
        MountRole::External
    } else {
        MountRole::SystemPathCandidate
    };

    MountClassification {
        filesystem_class,
        role,
        has_non_root_filesystem_root: mount.root != Path::new("/"),
    }
}

pub fn classify_mounts(
    mounts: &[MountRecord],
    user_home_directories: &[PathBuf],
) -> Vec<MountClassification> {
    mounts
        .iter()
        .map(|mount| classify_mount(mount, user_home_directories))
        .collect()
}

fn classify_filesystem(filesystem: &str) -> FilesystemClass {
    match filesystem {
        "proc" | "sysfs" | "devtmpfs" | "devpts" | "tmpfs" | "cgroup" | "cgroup2"
        | "pstore" | "securityfs" | "debugfs" | "tracefs" | "configfs" | "mqueue"
        | "hugetlbfs" | "rpc_pipefs" | "autofs" | "bpf" | "efivarfs" | "fusectl" => {
            FilesystemClass::Virtual
        }
        "nfs" | "nfs4" | "cifs" | "smb3" | "9p" | "ceph" | "glusterfs" | "afs"
        | "davfs" | "sshfs" => FilesystemClass::Network,
        "ext2" | "ext3" | "ext4" | "xfs" | "btrfs" | "vfat" | "fat" | "msdos"
        | "exfat" | "ntfs" | "ntfs3" | "f2fs" | "zfs" | "reiserfs" | "jfs"
        | "bcachefs" | "iso9660" => FilesystemClass::Local,
        _ => FilesystemClass::Unknown,
    }
}

fn is_boot_path(path: &Path) -> bool {
    ["/boot", "/efi"].iter().any(|base| {
        let base = Path::new(base);
        path == base || path.starts_with(base)
    })
}

fn is_external_path(path: &Path) -> bool {
    ["/mnt", "/media"].iter().any(|base| {
        let base = Path::new(base);
        path == base || path.starts_with(base)
    })
}

#[cfg(test)]
mod tests {
    use super::{
        FilesystemClass, MountRole, classify_mount, parse_mountinfo, parse_mountinfo_line,
    };
    use std::path::PathBuf;

    fn entry(root: &str, mount_point: &str, filesystem: &str) -> super::MountRecord {
        super::MountRecord {
            mount_id: 36,
            parent_id: 25,
            major_minor: "8:1".to_string(),
            root: PathBuf::from(root),
            mount_point: PathBuf::from(mount_point),
            mount_options: vec!["rw".to_string(), "relatime".to_string()],
            optional_fields: Vec::new(),
            filesystem: filesystem.to_string(),
            source: "/dev/sda1".to_string(),
            super_options: vec!["rw".to_string()],
        }
    }

    #[test]
    fn parses_mountinfo_fields_and_optional_metadata() {
        let parsed = parse_mountinfo_line(
            "36 25 8:1 / /boot rw,relatime shared:1 master:2 - ext4 /dev/sda1 rw",
        )
        .unwrap();

        assert_eq!(parsed.mount_id, 36);
        assert_eq!(parsed.parent_id, 25);
        assert_eq!(parsed.major_minor, "8:1");
        assert_eq!(parsed.root, PathBuf::from("/"));
        assert_eq!(parsed.mount_point, PathBuf::from("/boot"));
        assert_eq!(parsed.optional_fields, vec!["shared:1", "master:2"]);
        assert_eq!(parsed.filesystem, "ext4");
        assert_eq!(parsed.source, "/dev/sda1");
    }

    #[test]
    fn decodes_escaped_spaces_tabs_newlines_and_backslashes() {
        let parsed = parse_mountinfo_line(
            "36 25 8:1 / /media/Backup\\040Disk rw - ext4 /dev/mapper/backup\\134volume rw",
        )
        .unwrap();

        assert_eq!(parsed.mount_point, PathBuf::from("/media/Backup Disk"));
        assert_eq!(parsed.source, "/dev/mapper/backup\\volume");
    }

    #[test]
    fn rejects_malformed_mountinfo_instead_of_silently_dropping_entries() {
        let error = parse_mountinfo("not a valid mountinfo line").unwrap_err();
        assert!(error.contains("line 1"));
    }

    #[test]
    fn parses_multiple_mounts() {
        let parsed = parse_mountinfo(
            "36 25 8:1 / / rw - ext4 /dev/sda1 rw\n37 36 0:5 / /proc rw - proc proc rw",
        )
        .unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].filesystem, "proc");
    }

    #[test]
    fn classifies_virtual_and_network_filesystems_separately() {
        let homes = Vec::new();
        assert_eq!(
            classify_mount(&entry("/", "/proc", "proc"), &homes).filesystem_class,
            FilesystemClass::Virtual
        );
        assert_eq!(
            classify_mount(&entry("/", "/net/share", "nfs4"), &homes).filesystem_class,
            FilesystemClass::Network
        );
    }

    #[test]
    fn classifies_root_boot_and_external_mount_roles() {
        let homes = Vec::new();

        assert_eq!(
            classify_mount(&entry("/", "/", "ext4"), &homes).role,
            MountRole::SystemRoot
        );
        assert_eq!(
            classify_mount(&entry("/", "/boot/efi", "vfat"), &homes).role,
            MountRole::Boot
        );
        assert_eq!(
            classify_mount(&entry("/", "/media/backup", "ext4"), &homes).role,
            MountRole::External
        );
    }

    #[test]
    fn classifies_mounts_related_to_custom_home_paths_as_user_data() {
        let homes = vec![PathBuf::from("/data/users/alice")];

        assert_eq!(
            classify_mount(&entry("/", "/data/users", "ext4"), &homes).role,
            MountRole::UserData
        );
        assert_eq!(
            classify_mount(&entry("/", "/data/users/alice/archive", "ext4"), &homes).role,
            MountRole::UserData
        );
    }

    #[test]
    fn system_mounts_remain_candidates_instead_of_being_pruned() {
        let homes = vec![PathBuf::from("/home/alice")];

        for path in ["/usr", "/var", "/opt", "/srv"] {
            assert_eq!(
                classify_mount(&entry("/", path, "ext4"), &homes).role,
                MountRole::SystemPathCandidate
            );
        }
    }

    #[test]
    fn reports_non_root_filesystem_roots_without_assuming_bind_mounts() {
        let mount = entry("/subvolume", "/var/lib/example", "btrfs");
        let classification = classify_mount(&mount, &[]);

        assert!(classification.has_non_root_filesystem_root);
        assert_eq!(classification.filesystem_class, FilesystemClass::Local);
    }

    #[test]
    fn unknown_filesystems_require_later_policy_decisions() {
        assert_eq!(
            classify_mount(&entry("/", "/data", "fuse.custom"), &[]).filesystem_class,
            FilesystemClass::Unknown
        );
    }
}
