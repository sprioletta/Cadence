use adw::prelude::*;
use serde_json::Value;
use std::path::Path;
use std::process::Command;

const STORAGE_COLUMNS: [(&str, i32); 7] = [
    ("Device", 132),
    ("Type", 60),
    ("Filesystem", 90),
    ("Size (GB)", 90),
    ("Available (GB)", 100),
    ("Mounted at", 120),
    ("Destination check", 150),
];

fn main() {
    let application = adw::Application::builder()
        .application_id("io.github.sprioletta.Cadence")
        .build();

    application.connect_activate(build_ui);
    application.run();
}

fn build_ui(application: &adw::Application) {
    let header = adw::HeaderBar::builder()
        .title_widget(&gtk::Label::new(Some("Cadence")))
        .build();

    let stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(180)
        .build();

    stack.add_named(&make_overview_page(), Some("overview"));
    stack.add_named(
        &make_page(
            "document-save-symbolic",
            "Snapshots",
            "Browse and manage your system snapshots.",
        ),
        Some("snapshots"),
    );
    stack.add_named(&make_storage_page(), Some("storage"));
    stack.add_named(
        &make_page(
            "preferences-system-symbolic",
            "Settings",
            "Configure Cadence preferences.",
        ),
        Some("settings"),
    );
    stack.set_visible_child_name("overview");

    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 6);
    sidebar.set_width_request(220);
    sidebar.set_hexpand(false);
    sidebar.set_vexpand(true);
    sidebar.set_margin_top(12);
    sidebar.set_margin_bottom(12);
    sidebar.set_margin_start(12);
    sidebar.set_margin_end(12);

    let overview_button = add_nav_button(
        &sidebar,
        &stack,
        None,
        "Overview",
        "overview",
        "go-home-symbolic",
        true,
    );
    add_nav_button(
        &sidebar,
        &stack,
        Some(&overview_button),
        "Snapshots",
        "snapshots",
        "document-save-symbolic",
        false,
    );
    add_nav_button(
        &sidebar,
        &stack,
        Some(&overview_button),
        "Storage",
        "storage",
        "drive-removable-media-symbolic",
        false,
    );
    add_nav_button(
        &sidebar,
        &stack,
        Some(&overview_button),
        "Settings",
        "settings",
        "preferences-system-symbolic",
        false,
    );

    let separator = gtk::Separator::new(gtk::Orientation::Vertical);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.set_hexpand(true);
    content.set_vexpand(true);
    content.append(&sidebar);
    content.append(&separator);
    content.append(&stack);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));

    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("Cadence")
        .default_width(1000)
        .default_height(680)
        .content(&toolbar_view)
        .build();

    window.present();
}

fn make_overview_page() -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 20);
    page.set_hexpand(true);
    page.set_vexpand(true);
    page.set_margin_top(32);
    page.set_margin_bottom(32);
    page.set_margin_start(32);
    page.set_margin_end(32);

    let heading = gtk::Label::new(Some("System overview"));
    heading.add_css_class("title-1");
    heading.set_halign(gtk::Align::Start);

    let subtitle = gtk::Label::new(Some(
        "A summary of your backup destination and snapshot status.",
    ));
    subtitle.add_css_class("dim-label");
    subtitle.set_halign(gtk::Align::Start);
    subtitle.set_wrap(true);

    let grid = gtk::Grid::builder()
        .column_spacing(18)
        .row_spacing(18)
        .column_homogeneous(true)
        .hexpand(true)
        .build();

    grid.attach(
        &make_summary_card(
            "drive-harddisk-symbolic",
            "Backup destination",
            "Not configured",
            "Choose a destination to store your snapshots.",
        ),
        0,
        0,
        1,
        1,
    );
    grid.attach(
        &make_summary_card(
            "camera-photo-symbolic",
            "Latest snapshot",
            "No snapshots yet",
            "Your most recent snapshot will appear here.",
        ),
        1,
        0,
        1,
        1,
    );
    grid.attach(
        &make_summary_card(
            "drive-removable-media-symbolic",
            "Storage",
            "Not available",
            "Storage capacity will be shown when a destination is configured.",
        ),
        0,
        1,
        1,
        1,
    );
    grid.attach(
        &make_summary_card(
            "document-save-symbolic",
            "Last backup",
            "Never",
            "The date of your last completed backup will appear here.",
        ),
        1,
        1,
        1,
        1,
    );

    page.append(&heading);
    page.append(&subtitle);
    page.append(&grid);
    page
}

fn make_storage_page() -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 16);
    page.set_hexpand(true);
    page.set_vexpand(true);
    page.set_margin_top(32);
    page.set_margin_bottom(24);
    page.set_margin_start(32);
    page.set_margin_end(32);

    let heading = gtk::Label::new(Some("Storage devices"));
    heading.add_css_class("title-1");
    heading.set_halign(gtk::Align::Start);

    let subtitle = gtk::Label::new(Some(
        "Detected disks and partitions. This view is read-only; Cadence will not mount or modify devices.",
    ));
    subtitle.add_css_class("dim-label");
    subtitle.set_halign(gtk::Align::Start);
    subtitle.set_wrap(true);

    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    toolbar.set_hexpand(true);
    let status = gtk::Label::new(Some("Scanning storage devices…"));
    status.add_css_class("dim-label");
    status.set_halign(gtk::Align::Start);
    status.set_hexpand(true);

    let refresh = gtk::Button::with_label("Refresh");
    refresh.set_icon_name("view-refresh-symbolic");
    toolbar.append(&status);
    toolbar.append(&refresh);

    let table = gtk::Grid::new();
    table.set_column_spacing(12);
    table.set_row_spacing(0);
    table.set_hexpand(true);
    table.set_margin_start(12);
    table.set_margin_end(12);

    let scrolled = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&table)
        .build();

    page.append(&heading);
    page.append(&subtitle);
    page.append(&toolbar);
    page.append(&scrolled);

    let table_for_refresh = table.clone();
    let status_for_refresh = status.clone();
    refresh.connect_clicked(move |_| {
        refresh_storage(&table_for_refresh, &status_for_refresh);
    });

    refresh_storage(&table, &status);
    page
}

fn refresh_storage(table: &gtk::Grid, status: &gtk::Label) {
    while let Some(child) = table.first_child() {
        table.remove(&child);
    }
    append_storage_header(table);

    let output = Command::new("lsblk")
        .args([
            "--json",
            "--bytes",
            "--output",
            "NAME,PATH,TYPE,FSTYPE,SIZE,FSAVAIL,FSUSE%,MOUNTPOINTS,MODEL",
        ])
        .output();

    let output = match output {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            let error = String::from_utf8_lossy(&output.stderr);
            status.set_text(&format!(
                "Could not inspect storage devices: {}",
                error.trim()
            ));
            return;
        }
        Err(error) => {
            status.set_text(&format!("Could not run lsblk: {error}"));
            return;
        }
    };

    let document: Value = match serde_json::from_slice(&output.stdout) {
        Ok(document) => document,
        Err(error) => {
            status.set_text(&format!("Could not parse lsblk output: {error}"));
            return;
        }
    };

    let mut devices = Vec::new();
    if let Some(block_devices) = document.get("blockdevices").and_then(Value::as_array) {
        for device in block_devices {
            collect_devices(device, &mut devices);
        }
    }

    if devices.is_empty() {
        status.set_text("No storage devices or partitions were found.");
        return;
    }

    let count = devices.len();
    for (index, device) in devices.iter().enumerate() {
        append_device_row(table, device, index as i32 + 1);
    }
    status.set_text(&format!(
        "Found {count} devices and partitions. Information is read-only."
    ));
}

fn destination_status(device: &Value) -> &'static str {
    let Some(mountpoint) = device_mountpoint(device) else {
        return "Not mounted";
    };

    let filesystem = json_text(device, "fstype");
    let mount_options = mount_options_for(&mountpoint);
    let writable = write_access_for(&mountpoint);
    let available_bytes = device.get("fsavail").and_then(Value::as_u64);

    assess_destination(
        filesystem.as_deref(),
        Some(&mountpoint),
        mount_options.as_deref(),
        writable,
        available_bytes,
    )
}

fn mount_options_for(mountpoint: &str) -> Option<String> {
    let output = Command::new("findmnt")
        .args([
            "--noheadings",
            "--output",
            "OPTIONS",
            "--target",
            mountpoint,
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let options = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!options.is_empty()).then_some(options)
}

fn write_access_for(mountpoint: &str) -> Option<bool> {
    Command::new("test")
        .arg("-w")
        .arg(mountpoint)
        .status()
        .ok()
        .map(|status| status.success())
}

fn assess_destination(
    filesystem: Option<&str>,
    mountpoint: Option<&str>,
    mount_options: Option<&str>,
    writable: Option<bool>,
    available_bytes: Option<u64>,
) -> &'static str {
    let Some(mountpoint) = mountpoint.filter(|path| !path.is_empty()) else {
        return "Not mounted";
    };

    if mountpoint == "/" {
        return "System root";
    }

    let Some(options) = mount_options else {
        return "Mount not verified";
    };

    if options.split(',').any(|option| option == "ro") {
        return "Read-only";
    }
    if !options.split(',').any(|option| option == "rw") {
        return "Mount mode unknown";
    }

    match writable {
        Some(true) => {}
        Some(false) => return "No write access",
        None => return "Access not verified",
    }

    let filesystem = filesystem.unwrap_or_default().to_ascii_lowercase();
    if !matches!(
        filesystem.as_str(),
        "ext2" | "ext3" | "ext4" | "btrfs" | "xfs" | "f2fs"
    ) {
        return "Verify filesystem";
    }

    match available_bytes {
        Some(0) => "No free space",
        Some(_) => "Candidate",
        None => "Check free space",
    }
}

fn device_mountpoint(device: &Value) -> Option<String> {
    device
        .get("mountpoints")
        .and_then(Value::as_array)
        .and_then(|points| {
            points
                .iter()
                .filter_map(Value::as_str)
                .find(|point| !point.is_empty())
        })
        .map(str::to_string)
        .or_else(|| {
            device
                .get("mountpoints")
                .and_then(Value::as_str)
                .filter(|point| !point.is_empty())
                .map(str::to_string)
        })
}

fn collect_devices(device: &Value, devices: &mut Vec<Value>) {
    devices.push(device.clone());
    if let Some(children) = device.get("children").and_then(Value::as_array) {
        for child in children {
            collect_devices(child, devices);
        }
    }
}

fn append_storage_header(table: &gtk::Grid) {
    for (column, (title, width)) in STORAGE_COLUMNS.iter().enumerate() {
        let label = gtk::Label::new(Some(title));
        label.add_css_class("heading");
        let numeric_column = column == 3 || column == 4;
        label.set_halign(if numeric_column {
            gtk::Align::End
        } else {
            gtk::Align::Start
        });
        label.set_xalign(if numeric_column { 1.0 } else { 0.0 });
        label.set_width_request(*width);
        label.set_hexpand(column == STORAGE_COLUMNS.len() - 1);
        label.set_margin_top(8);
        label.set_margin_bottom(8);
        table.attach(&label, column as i32, 0, 1, 1);
    }
}

fn append_device_row(table: &gtk::Grid, device: &Value, row_index: i32) {
    let path = json_text(device, "path")
        .or_else(|| json_text(device, "name"))
        .unwrap_or_else(|| "Unknown".to_string());
    let kind = json_text(device, "type").unwrap_or_else(|| "—".to_string());
    let filesystem = json_text(device, "fstype").unwrap_or_else(|| "—".to_string());
    let size = device
        .get("size")
        .and_then(Value::as_u64)
        .map(format_gigabytes)
        .unwrap_or_else(|| "—".to_string());
    let available = device
        .get("fsavail")
        .and_then(Value::as_u64)
        .map(format_gigabytes)
        .unwrap_or_else(|| "—".to_string());
    let mountpoint = device_mountpoint(device).unwrap_or_else(|| "—".to_string());

    let mountpoint_display = if mountpoint.starts_with('/') && mountpoint != "/" {
        Path::new(&mountpoint)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(&mountpoint)
            .to_string()
    } else {
        mountpoint.clone()
    };

    let destination = destination_status(device).to_string();
    let values = [
        path,
        kind,
        filesystem,
        size,
        available,
        mountpoint_display,
        destination,
    ];
    for (column, (value, (_, width))) in values.into_iter().zip(STORAGE_COLUMNS.iter()).enumerate()
    {
        let label = gtk::Label::new(Some(&value));
        label.set_halign(if column == 3 || column == 4 {
            gtk::Align::End
        } else {
            gtk::Align::Start
        });
        label.set_xalign(if column == 3 || column == 4 { 1.0 } else { 0.0 });
        if column == 3 || column == 4 {
            label.add_css_class("numeric");
        }
        label.set_width_request(*width);
        label.set_hexpand(column == STORAGE_COLUMNS.len() - 1);
        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);

        if column == 5 && mountpoint != "—" {
            label.set_tooltip_text(Some(&mountpoint));
        }
        if column == 6 {
            let explanation = match value.as_str() {
                "Candidate" => {
                    "Preliminary candidate only. Final backup preflight is still required."
                }
                "System root" => {
                    "The running system root is not accepted as a backup destination."
                }
                "Verify filesystem" => {
                    "Filesystem support for the required snapshot features is not confirmed."
                }
                "Check free space" => "Available space could not be determined.",
                "Mount not verified" => "The mount status could not be verified.",
                "Access not verified" => "Write access could not be checked.",
                "Mount mode unknown" => {
                    "The mount options do not clearly indicate read-write access."
                }
                "No write access" => {
                    "The current user does not have write access to this mountpoint."
                }
                "No free space" => "The filesystem reports no available space.",
                "Read-only" => "The filesystem is mounted read-only.",
                _ => "The filesystem is not mounted.",
            };
            label.set_tooltip_text(Some(explanation));
        }
        label.set_margin_top(10);
        label.set_margin_bottom(10);
        table.attach(&label, column as i32, row_index, 1, 1);
    }
}

fn json_text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn format_gigabytes(bytes: u64) -> String {
    let gigabytes = bytes as f64 / 1_000_000_000.0;
    if gigabytes > 0.0 && gigabytes < 0.1 {
        "<0.1".to_string()
    } else {
        format!("{gigabytes:.1}")
    }
}

fn make_summary_card(icon_name: &str, title: &str, value: &str, description: &str) -> gtk::Frame {
    let frame = gtk::Frame::new(None);
    frame.add_css_class("card");
    frame.set_hexpand(true);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(20);
    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("heading");
    title_label.set_halign(gtk::Align::Start);
    header.append(&icon);
    header.append(&title_label);

    let value_label = gtk::Label::new(Some(value));
    value_label.add_css_class("title-2");
    value_label.set_halign(gtk::Align::Start);
    value_label.set_wrap(true);

    let description_label = gtk::Label::new(Some(description));
    description_label.add_css_class("dim-label");
    description_label.set_halign(gtk::Align::Start);
    description_label.set_wrap(true);
    description_label.set_xalign(0.0);

    content.append(&header);
    content.append(&value_label);
    content.append(&description_label);
    frame.set_child(Some(&content));
    frame
}

fn make_page(icon: &str, title: &str, description: &str) -> adw::StatusPage {
    adw::StatusPage::builder()
        .icon_name(icon)
        .title(title)
        .description(description)
        .build()
}

fn add_nav_button(
    sidebar: &gtk::Box,
    stack: &gtk::Stack,
    group: Option<&gtk::ToggleButton>,
    label: &str,
    page: &str,
    icon_name: &str,
    active: bool,
) -> gtk::ToggleButton {
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(18);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_margin_top(6);
    row.set_margin_bottom(6);
    row.set_margin_start(8);
    row.set_margin_end(8);
    row.append(&icon);
    row.append(&gtk::Label::new(Some(label)));

    let button = gtk::ToggleButton::new();
    button.set_child(Some(&row));
    button.set_hexpand(true);
    button.set_halign(gtk::Align::Fill);
    if let Some(group_button) = group {
        button.set_group(Some(group_button));
    }
    button.set_active(active);

    let stack_clone = stack.clone();
    let page_name = page.to_string();
    button.connect_clicked(move |_| {
        stack_clone.set_visible_child_name(&page_name);
    });

    sidebar.append(&button);
    button
}

#[cfg(test)]
mod tests {
    use super::assess_destination;

    #[test]
    fn destination_requires_a_mountpoint() {
        assert_eq!(
            assess_destination(None, None, None, None, None),
            "Not mounted"
        );
    }

    #[test]
    fn system_root_is_not_a_backup_destination() {
        assert_eq!(
            assess_destination(
                Some("ext4"),
                Some("/"),
                Some("rw,relatime"),
                Some(true),
                Some(1_000_000_000),
            ),
            "System root"
        );
    }

    #[test]
    fn read_only_mount_is_rejected() {
        assert_eq!(
            assess_destination(
                Some("ext4"),
                Some("/mnt/backup"),
                Some("ro,relatime"),
                Some(false),
                Some(1_000_000_000),
            ),
            "Read-only"
        );
    }

    #[test]
    fn destination_requires_write_access() {
        assert_eq!(
            assess_destination(
                Some("ext4"),
                Some("/mnt/backup"),
                Some("rw,relatime"),
                Some(false),
                Some(1_000_000_000),
            ),
            "No write access"
        );
    }

    #[test]
    fn unknown_filesystems_require_verification() {
        assert_eq!(
            assess_destination(
                Some("exfat"),
                Some("/mnt/backup"),
                Some("rw,relatime"),
                Some(true),
                Some(1_000_000_000),
            ),
            "Verify filesystem"
        );
    }

    #[test]
    fn destination_without_reported_space_is_not_confirmed() {
        assert_eq!(
            assess_destination(
                Some("ext4"),
                Some("/mnt/backup"),
                Some("rw,relatime"),
                Some(true),
                None,
            ),
            "Check free space"
        );
    }

    #[test]
    fn writable_linux_filesystem_with_space_is_a_candidate() {
        assert_eq!(
            assess_destination(
                Some("ext4"),
                Some("/mnt/backup"),
                Some("rw,relatime"),
                Some(true),
                Some(1_000_000_000),
            ),
            "Candidate"
        );
    }

    #[test]
    fn zero_available_space_is_rejected() {
        assert_eq!(
            assess_destination(
                Some("ext4"),
                Some("/mnt/backup"),
                Some("rw,relatime"),
                Some(true),
                Some(0),
            ),
            "No free space"
        );
    }

    #[test]
    fn unknown_mount_mode_is_not_accepted() {
        assert_eq!(
            assess_destination(
                Some("ext4"),
                Some("/mnt/backup"),
                Some("relatime"),
                Some(true),
                Some(1_000_000_000),
            ),
            "Mount mode unknown"
        );
    }
}
