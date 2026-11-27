use adw::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;

const STORAGE_COLUMNS: [(&str, i32); 7] = [
    ("Device", 132),
    ("Type", 60),
    ("Filesystem", 90),
    ("Size (GB)", 90),
    ("Available (GB)", 100),
    ("Mount", 76),
    ("Destination", 104),
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
    // Keep the window large enough for the sidebar and the widest tab content.
    // The window remains resizable, but cannot be shrunk until its tabs are clipped.
    content.set_size_request(1080, 560);
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
    let sort_state = Rc::new(RefCell::new(None));

    let scrolled = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .child(&table)
        .build();
    scrolled.set_propagate_natural_height(true);
    scrolled.set_max_content_height(440);

    let device_card = gtk::Frame::new(None);
    device_card.add_css_class("card");
    device_card.set_hexpand(true);
    device_card.set_vexpand(false);

    let card_content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card_content.set_margin_top(12);
    card_content.set_margin_bottom(12);
    card_content.set_margin_start(16);
    card_content.set_margin_end(16);

    let card_heading = gtk::Label::new(Some("Detected volumes"));
    card_heading.add_css_class("heading");
    card_heading.set_halign(gtk::Align::Start);
    card_content.append(&card_heading);
    card_content.append(&scrolled);
    device_card.set_child(Some(&card_content));

    page.append(&heading);
    page.append(&subtitle);
    page.append(&toolbar);
    page.append(&device_card);

    let table_for_refresh = table.clone();
    let status_for_refresh = status.clone();
    let sort_for_refresh = sort_state.clone();
    refresh.connect_clicked(move |_| {
        refresh_storage(&table_for_refresh, &status_for_refresh, &sort_for_refresh);
    });

    refresh_storage(&table, &status, &sort_state);
    page
}

fn refresh_storage(
    table: &gtk::Grid,
    status: &gtk::Label,
    sort_state: &Rc<RefCell<Option<(usize, bool)>>>,
) {
    while let Some(child) = table.first_child() {
        table.remove(&child);
    }
    let sort = *sort_state.borrow();
    append_storage_header(table, sort, table, status, sort_state);

    let output = Command::new("lsblk")
        .args([
            "--json",
            "--bytes",
            "--output",
            "NAME,PATH,TYPE,FSTYPE,PARTTYPE,PARTLABEL,SIZE,FSAVAIL,FSUSE%,MOUNTPOINTS,MODEL",
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

    if let Some((column, ascending)) = sort {
        devices.sort_by(|left, right| {
            let ordering = compare_device_values(left, right, column);
            let ordering = if ascending { ordering } else { ordering.reverse() };
            ordering.then_with(|| compare_device_values(left, right, 0))
        });
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
    let kind = json_text(device, "type");
    let filesystem = json_text(device, "fstype");
    let partition_type = json_text(device, "parttype");
    let partition_label = json_text(device, "partlabel");
    let mountpoint = device_mountpoint(device);
    let mount_options = mountpoint.as_deref().and_then(mount_options_for);
    let available_bytes = device.get("fsavail").and_then(Value::as_u64);

    assess_destination(
        kind.as_deref(),
        filesystem.as_deref(),
        partition_type.as_deref(),
        partition_label.as_deref(),
        mountpoint.as_deref(),
        mount_options.as_deref(),
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

fn assess_destination(
    device_type: Option<&str>,
    filesystem: Option<&str>,
    partition_type: Option<&str>,
    partition_label: Option<&str>,
    mountpoint: Option<&str>,
    mount_options: Option<&str>,
    available_bytes: Option<u64>,
) -> &'static str {
    let device_type = device_type.unwrap_or_default().to_ascii_lowercase();
    if !matches!(device_type.as_str(), "disk" | "part" | "lvm" | "crypt") {
        return "Unsupported volume type";
    }

    let filesystem = filesystem.unwrap_or_default().to_ascii_lowercase();
    if filesystem == "swap" {
        return "Swap volume";
    }

    let partition_type = partition_type.unwrap_or_default().to_ascii_lowercase();
    let partition_label = partition_label.unwrap_or_default().to_ascii_lowercase();
    if partition_type == "c12a7328-f81f-11d2-ba4b-00a0c93ec93b"
        || partition_type == "0xef"
        || partition_label.contains("efi system")
        || partition_label.contains("bios boot")
        || partition_label.contains("system reserved")
        || partition_type == "21686148-6449-6e6f-744e-656564454649"
        || partition_type == "e3c9e316-0b5c-4db8-817d-f92df00215ae"
        || partition_type == "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f"
        || partition_type == "0x82"
    {
        return "Boot/system partition";
    }

    if matches!(
        filesystem.as_str(),
        "lvm2_member" | "linux_raid_member" | "crypto_luks"
    ) {
        return "Volume container";
    }

    let Some(mountpoint) = mountpoint.filter(|path| !path.is_empty()) else {
        return "Not mounted";
    };

    if mountpoint == "/" {
        return "System root";
    }

    if matches!(
        mountpoint,
        "/boot" | "/boot/efi" | "/boot/firmware" | "/efi"
    ) {
        return "Boot volume";
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

    if !matches!(
        filesystem.as_str(),
        "ext2" | "ext3" | "ext4" | "btrfs" | "xfs" | "f2fs"
    ) {
        return "Verify filesystem";
    }

    match available_bytes {
        Some(0) => "No free space",
        Some(_) => "Potential device",
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

fn append_storage_header(
    table: &gtk::Grid,
    sort: Option<(usize, bool)>,
    table_for_sort: &gtk::Grid,
    status_for_sort: &gtk::Label,
    sort_state: &Rc<RefCell<Option<(usize, bool)>>>,
) {
    for (column, (title, width)) in STORAGE_COLUMNS.iter().enumerate() {
        let label = if let Some((sorted_column, ascending)) = sort {
            if sorted_column == column {
                format!("{title} {}", if ascending { "↑" } else { "↓" })
            } else {
                (*title).to_string()
            }
        } else {
            (*title).to_string()
        };

        let button = gtk::Button::with_label(&label);
        button.add_css_class("flat");
        button.add_css_class("heading");
        button.set_halign(gtk::Align::Fill);
        button.set_hexpand(false);
        button.set_size_request(*width, -1);
        button.set_margin_top(4);
        button.set_margin_bottom(4);
        button.set_tooltip_text(Some(&format!("Sort by {title}")));

        let table_weak = table_for_sort.downgrade();
        let status_weak = status_for_sort.downgrade();
        let sort_state = sort_state.clone();
        button.connect_clicked(move |_| {
            if let (Some(table), Some(status)) = (table_weak.upgrade(), status_weak.upgrade()) {
                let next_sort = match *sort_state.borrow() {
                    Some((current_column, ascending)) if current_column == column => {
                        Some((column, !ascending))
                    }
                    _ => Some((column, true)),
                };
                *sort_state.borrow_mut() = next_sort;
                refresh_storage(&table, &status, &sort_state);
            }
        });

        table.attach(&button, column as i32, 0, 1, 1);
    }
}

fn compare_device_values(left: &Value, right: &Value, column: usize) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    match column {
        0 => compare_optional_text(json_text(left, "path").or_else(|| json_text(left, "name")),
            json_text(right, "path").or_else(|| json_text(right, "name"))),
        1 => compare_optional_text(json_text(left, "type"), json_text(right, "type")),
        2 => compare_optional_text(json_text(left, "fstype"), json_text(right, "fstype")),
        3 => compare_optional_number(left.get("size").and_then(Value::as_u64), right.get("size").and_then(Value::as_u64)),
        4 => compare_optional_number(left.get("fsavail").and_then(Value::as_u64), right.get("fsavail").and_then(Value::as_u64)),
        5 => compare_optional_text(device_mountpoint(left), device_mountpoint(right)),
        6 => destination_status(left).cmp(destination_status(right)),
        _ => Ordering::Equal,
    }
}

fn compare_optional_text(left: Option<String>, right: Option<String>) -> std::cmp::Ordering {
    left.unwrap_or_default().to_lowercase().cmp(&right.unwrap_or_default().to_lowercase())
}

fn compare_optional_number(left: Option<u64>, right: Option<u64>) -> std::cmp::Ordering {
    left.unwrap_or(0).cmp(&right.unwrap_or(0))
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
    let mountpoint = device_mountpoint(device);
    let destination = destination_status(device);

    let text_values = [
        path,
        kind,
        filesystem,
        size,
        available,
    ];

    for (column, value) in text_values.into_iter().enumerate() {
        let label = gtk::Label::new(Some(&value));
        let numeric = column == 3 || column == 4;
        label.set_halign(if numeric { gtk::Align::End } else { gtk::Align::Start });
        label.set_xalign(if numeric { 1.0 } else { 0.0 });
        if numeric {
            label.add_css_class("numeric");
        }
        label.set_size_request(STORAGE_COLUMNS[column].1, -1);
        label.set_hexpand(false);
        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        label.set_margin_top(8);
        label.set_margin_bottom(8);
        table.attach(&label, column as i32, row_index, 1, 1);
    }

    let mountpoint_text = mountpoint.as_deref().unwrap_or("Not mounted");
    let mount_description = match mountpoint.as_deref() {
        Some(path) => format!("This volume is mounted at:\n{path}"),
        None => "This volume is not currently mounted.".to_string(),
    };
    let mount_icon = if mountpoint.is_some() {
        "folder-symbolic"
    } else {
        "drive-harddisk-symbolic"
    };
    let mount_button = make_detail_button(
        mount_icon,
        "Show mount information",
        "Mount information",
        &mount_description,
    );
    mount_button.set_size_request(STORAGE_COLUMNS[5].1, -1);
    mount_button.set_hexpand(false);
    mount_button.set_halign(gtk::Align::Start);
    mount_button.set_margin_top(4);
    mount_button.set_margin_bottom(4);
    mount_button.set_tooltip_text(Some(mountpoint_text));
    table.attach(&mount_button, 5, row_index, 1, 1);

    let explanation = destination_explanation(destination);
    let (status_icon, _status_css) = destination_icon(destination);
    let status_button = make_detail_button(
        status_icon,
        "Show destination assessment details",
        destination,
        explanation,
    );
    status_button.set_size_request(STORAGE_COLUMNS[6].1, -1);
    status_button.set_hexpand(false);
    status_button.set_halign(gtk::Align::Start);
    status_button.set_margin_top(4);
    status_button.set_margin_bottom(4);
    table.attach(&status_button, 6, row_index, 1, 1);
}

fn make_detail_button(
    icon_name: &str,
    tooltip: &str,
    title: &str,
    description: &str,
) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.set_icon_name(icon_name);
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");

    let popover = gtk::Popover::new();
    popover.set_has_arrow(true);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(14);
    content.set_margin_end(14);
    content.set_width_request(260);

    let heading = gtk::Label::new(Some(title));
    heading.add_css_class("heading");
    heading.set_halign(gtk::Align::Start);
    heading.set_wrap(true);
    heading.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    heading.set_width_chars(24);
    heading.set_max_width_chars(34);

    let details = gtk::Label::new(Some(description));
    details.set_halign(gtk::Align::Start);
    details.set_xalign(0.0);
    details.set_wrap(true);
    details.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    details.set_width_chars(32);
    details.set_max_width_chars(40);
    details.set_selectable(true);

    content.append(&heading);
    content.append(&details);
    popover.set_child(Some(&content));
    button.set_popover(Some(&popover));
    button
}

fn destination_icon(status: &str) -> (&'static str, &'static str) {
    match status {
        "Potential device" => ("dialog-information-symbolic", "suggested-action"),
        "Unsupported volume type"
        | "Swap volume"
        | "Boot/system partition"
        | "Boot volume"
        | "Volume container"
        | "System root"
        | "Read-only"
        | "No free space" => ("dialog-error-symbolic", "error"),
        _ => ("dialog-warning-symbolic", "warning"),
    }
}

fn destination_explanation(status: &str) -> &'static str {
    match status {
        "Potential device" => {
            "The volume type and filesystem appear suitable. Select a destination directory before checking write permissions and whether this filesystem also contains source data."
        }
        "Unsupported volume type" => {
            "This block-device type is not eligible as a backup destination."
        }
        "Swap volume" => "Swap space is never a backup destination.",
        "Boot/system partition" => {
            "EFI, BIOS boot, reserved and swap partition types are excluded."
        }
        "Boot volume" => "Boot and EFI mount points are excluded from backup destinations.",
        "Volume container" => {
            "This is a storage container or RAID/LUKS member, not a mounted data filesystem."
        }
        "System root" => "The running system root is not accepted as a backup destination.",
        "Verify filesystem" => {
            "Filesystem support for the required snapshot features is not confirmed."
        }
        "Check free space" => "Available space could not be determined.",
        "Mount not verified" => "The mount status could not be verified.",
        "Mount mode unknown" => {
            "The mount options do not clearly indicate read-write access."
        }
        "No free space" => "The filesystem reports no available space.",
        "Read-only" => "The filesystem is mounted read-only.",
        _ => "The filesystem is not mounted.",
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

    fn assess(
        device_type: Option<&str>,
        filesystem: Option<&str>,
        partition_type: Option<&str>,
        partition_label: Option<&str>,
        mountpoint: Option<&str>,
        options: Option<&str>,
        available: Option<u64>,
    ) -> &'static str {
        assess_destination(
            device_type,
            filesystem,
            partition_type,
            partition_label,
            mountpoint,
            options,
            available,
        )
    }

    #[test]
    fn swap_filesystem_is_rejected_even_when_unmounted() {
        assert_eq!(
            assess(Some("part"), Some("swap"), None, None, None, None, None),
            "Swap volume"
        );
    }

    #[test]
    fn efi_partition_is_rejected_even_when_unmounted() {
        assert_eq!(
            assess(
                Some("part"),
                Some("vfat"),
                Some("c12a7328-f81f-11d2-ba4b-00a0c93ec93b"),
                Some("EFI System Partition"),
                None,
                None,
                None,
            ),
            "Boot/system partition"
        );
    }

    #[test]
    fn mbr_efi_and_swap_partition_types_are_rejected() {
        for partition_type in ["0xef", "0x82"] {
            assert_eq!(
                assess(
                    Some("part"),
                    Some("vfat"),
                    Some(partition_type),
                    None,
                    None,
                    None,
                    None,
                ),
                "Boot/system partition"
            );
        }
    }

    #[test]
    fn bios_boot_partition_is_rejected() {
        assert_eq!(
            assess(
                Some("part"),
                None,
                Some("21686148-6449-6e6f-744e-656564454649"),
                None,
                None,
                None,
                None,
            ),
            "Boot/system partition"
        );
    }

    #[test]
    fn boot_mountpoints_are_rejected() {
        for mountpoint in ["/boot", "/boot/efi", "/boot/firmware", "/efi"] {
            assert_eq!(
                assess(
                    Some("part"),
                    Some("ext4"),
                    None,
                    None,
                    Some(mountpoint),
                    Some("rw,relatime"),
                    Some(1_000_000_000),
                ),
                "Boot volume"
            );
        }
    }

    #[test]
    fn system_root_is_rejected() {
        assert_eq!(
            assess(
                Some("part"),
                Some("ext4"),
                None,
                None,
                Some("/"),
                Some("rw,relatime"),
                Some(1_000_000_000),
            ),
            "System root"
        );
    }

    #[test]
    fn unsupported_block_device_types_are_rejected() {
        assert_eq!(
            assess(
                Some("loop"),
                Some("ext4"),
                None,
                None,
                Some("/mnt/backup"),
                Some("rw,relatime"),
                Some(1_000_000_000),
            ),
            "Unsupported volume type"
        );
    }

    #[test]
    fn storage_containers_are_not_destinations() {
        for filesystem in ["LVM2_member", "linux_raid_member", "crypto_LUKS"] {
            assert_eq!(
                assess(Some("part"), Some(filesystem), None, None, None, None, None,),
                "Volume container"
            );
        }
    }

    #[test]
    fn read_only_mount_is_rejected() {
        assert_eq!(
            assess(
                Some("part"),
                Some("ext4"),
                None,
                None,
                Some("/mnt/backup"),
                Some("ro,relatime"),
                Some(1_000_000_000),
            ),
            "Read-only"
        );
    }

    #[test]
    fn unknown_filesystems_require_verification() {
        assert_eq!(
            assess(
                Some("part"),
                Some("exfat"),
                None,
                None,
                Some("/mnt/backup"),
                Some("rw,relatime"),
                Some(1_000_000_000),
            ),
            "Verify filesystem"
        );
    }

    #[test]
    fn mount_mode_must_be_verified() {
        assert_eq!(
            assess(
                Some("part"),
                Some("ext4"),
                None,
                None,
                Some("/mnt/backup"),
                Some("relatime"),
                Some(1_000_000_000),
            ),
            "Mount mode unknown"
        );
    }

    #[test]
    fn missing_space_information_is_not_confirmed() {
        assert_eq!(
            assess(
                Some("part"),
                Some("ext4"),
                None,
                None,
                Some("/mnt/backup"),
                Some("rw,relatime"),
                None,
            ),
            "Check free space"
        );
    }

    #[test]
    fn zero_available_space_is_rejected() {
        assert_eq!(
            assess(
                Some("part"),
                Some("ext4"),
                None,
                None,
                Some("/mnt/backup"),
                Some("rw,relatime"),
                Some(0),
            ),
            "No free space"
        );
    }

    #[test]
    fn supported_data_volume_is_only_a_potential_device() {
        assert_eq!(
            assess(
                Some("part"),
                Some("ext4"),
                None,
                None,
                Some("/mnt/backup"),
                Some("rw,relatime"),
                Some(1_000_000_000),
            ),
            "Potential device"
        );
    }
}
