use adw::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Eq)]
struct DestinationConfig {
    mountpoint: String,
    path: String,
    uuid: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BackupConfig {
    system_enabled: bool,
    user_data_enabled: bool,
    user_data_mode: String,
    user_selection_mode: String,
    selected_users: Vec<String>,
    custom_exclusions: Vec<String>,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            system_enabled: true,
            user_data_enabled: true,
            user_data_mode: "all".to_string(),
            user_selection_mode: "all".to_string(),
            selected_users: Vec::new(),
            custom_exclusions: Vec::new(),
        }
    }
}

impl BackupConfig {
    fn from_json(value: &Value) -> Self {
        let defaults = Self::default();
        let Some(backup) = value.get("backup") else {
            return defaults;
        };
        let user_data_mode = backup
            .get("user_data")
            .and_then(|value| value.get("include_mode"))
            .and_then(Value::as_str)
            .filter(|mode| matches!(*mode, "all" | "hidden_only" | "none"))
            .unwrap_or("all")
            .to_string();
        let selected_users: Vec<String> = backup
            .get("user_data")
            .and_then(|value| value.get("home_directories"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        // Older configurations had no explicit selection mode. An empty legacy
        // list now migrates to "all"; a populated list migrates to "selected".
        let user_selection_mode = backup
            .get("user_data")
            .and_then(|value| value.get("selection_mode"))
            .and_then(Value::as_str)
            .filter(|mode| matches!(*mode, "all" | "selected"))
            .map(str::to_string)
            .unwrap_or_else(|| {
                if selected_users.is_empty() {
                    "all".to_string()
                } else {
                    "selected".to_string()
                }
            });
        let custom_exclusions = backup
            .get("exclusions")
            .and_then(|value| value.get("custom_paths"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            system_enabled: backup
                .get("system")
                .and_then(|value| value.get("enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(defaults.system_enabled),
            user_data_enabled: backup
                .get("user_data")
                .and_then(|value| value.get("enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(defaults.user_data_enabled),
            user_data_mode,
            user_selection_mode,
            selected_users,
            custom_exclusions,
        }
    }

    fn to_json(&self) -> Value {
        serde_json::json!({
            "mode": "system",
            "method": "rsync",
            "system": {
                "enabled": self.system_enabled,
                "sources": ["/"]
            },
            "user_data": {
                "enabled": self.user_data_enabled,
                "include_mode": self.user_data_mode,
                "selection_mode": self.user_selection_mode,
                "home_directories": self.selected_users
            },
            "exclusions": {
                "system_defaults": true,
                "custom_paths": self.custom_exclusions
            }
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalUser {
    username: String,
    uid: u32,
    home: PathBuf,
}

fn discover_local_users() -> Result<Vec<LocalUser>, String> {
    let passwd = fs::read_to_string("/etc/passwd")
        .map_err(|error| format!("Could not read /etc/passwd: {error}"))?;
    let login_defs = fs::read_to_string("/etc/login.defs")
        .map_err(|error| format!("Could not read /etc/login.defs: {error}"))?;
    let shells_file = fs::read_to_string("/etc/shells")
        .map_err(|error| format!("Could not read /etc/shells: {error}"))?;

    let uid_min = configured_uid_min(&login_defs).unwrap_or(1000);
    let shells = parse_login_shells(&shells_file);
    Ok(parse_local_users(&passwd, uid_min, &shells, |home| {
        home.is_dir()
    }))
}

fn configured_uid_min(contents: &str) -> Option<u32> {
    contents.lines().find_map(|line| {
        let line = line.split('#').next()?.trim();
        let mut fields = line.split_whitespace();
        if fields.next()? != "UID_MIN" {
            return None;
        }
        fields.next()?.parse().ok()
    })
}

fn parse_login_shells(contents: &str) -> Vec<String> {
    contents
        .lines()
        .filter_map(|line| {
            let shell = line.split('#').next()?.trim();
            if shell.starts_with('/') {
                Some(shell.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn parse_local_users(
    passwd: &str,
    uid_min: u32,
    shells: &[String],
    home_is_directory: impl Fn(&Path) -> bool,
) -> Vec<LocalUser> {
    let mut users = Vec::new();

    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() != 7 {
            continue;
        }

        let username = fields[0];
        let Ok(uid) = fields[2].parse::<u32>() else {
            continue;
        };
        let home = Path::new(fields[5]);

        if username.is_empty()
            || username == "root"
            || uid < uid_min
            || !home.is_absolute()
            || !home_is_directory(home)
            || !shells.iter().any(|shell| shell == fields[6])
        {
            continue;
        }

        users.push(LocalUser {
            username: username.to_string(),
            uid,
            home: home.to_path_buf(),
        });
    }

    users.sort_by(|left, right| left.username.cmp(&right.username));
    users.dedup_by(|left, right| left.username == right.username || left.uid == right.uid);
    users
}

fn destination_from_json(value: &Value) -> Option<DestinationConfig> {
    let destination = value.get("destination").unwrap_or(value);
    let mountpoint = destination.get("mountpoint")?.as_str()?.to_string();
    let path = destination.get("path")?.as_str()?.to_string();
    if Path::new(&path) != destination_path_for(&mountpoint) {
        return None;
    }
    let uuid = destination
        .get("uuid")
        .and_then(Value::as_str)
        .filter(|uuid| !uuid.trim().is_empty())
        .map(str::to_string);
    Some(DestinationConfig {
        mountpoint,
        path,
        uuid,
    })
}

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
    stack.add_named(&make_settings_page(), Some("settings"));
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
        .default_width(1150)
        .default_height(740)
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

    let destination_config = Rc::new(RefCell::new(load_destination_config()));
    let destination_info = gtk::Label::new(None);
    destination_info.set_halign(gtk::Align::Start);
    destination_info.set_xalign(0.0);
    destination_info.set_wrap(true);
    destination_info.add_css_class("dim-label");
    destination_info.set_text("Checking active backup destination…");

    let destination_card = gtk::Frame::new(None);
    destination_card.add_css_class("card");
    destination_card.set_hexpand(true);
    let destination_content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    destination_content.set_margin_top(12);
    destination_content.set_margin_bottom(12);
    destination_content.set_margin_start(16);
    destination_content.set_margin_end(16);
    let destination_heading = gtk::Label::new(Some("Backup destination"));
    destination_heading.add_css_class("heading");
    destination_heading.set_halign(gtk::Align::Start);
    destination_content.append(&destination_heading);
    destination_content.append(&destination_info);
    destination_card.set_child(Some(&destination_content));

    let destination_result = gtk::Label::new(Some("No destination operation performed."));
    destination_result.set_halign(gtk::Align::Start);
    destination_result.set_xalign(0.0);
    destination_result.set_wrap(true);
    destination_result.add_css_class("dim-label");

    let result_card = gtk::Frame::new(None);
    result_card.add_css_class("card");
    result_card.set_hexpand(true);
    let result_content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    result_content.set_margin_top(12);
    result_content.set_margin_bottom(12);
    result_content.set_margin_start(16);
    result_content.set_margin_end(16);
    let result_heading = gtk::Label::new(Some("Destination status"));
    result_heading.add_css_class("heading");
    result_heading.set_halign(gtk::Align::Start);
    result_content.append(&result_heading);
    result_content.append(&destination_result);
    result_card.set_child(Some(&result_content));

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
    // Keep automatic scrolling for long device lists, but avoid GTK overlay
    // scrollbars appearing only when the pointer moves over the table.
    scrolled.set_overlay_scrolling(false);
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
    page.append(&destination_card);
    page.append(&result_card);
    page.append(&device_card);

    let table_for_refresh = table.clone();
    let status_for_refresh = status.clone();
    let sort_for_refresh = sort_state.clone();
    let config_for_refresh = destination_config.clone();
    let info_for_refresh = destination_info.clone();
    let result_for_refresh = destination_result.clone();
    refresh.connect_clicked(move |_| {
        refresh_storage(
            &table_for_refresh,
            &status_for_refresh,
            &sort_for_refresh,
            &config_for_refresh,
            &info_for_refresh,
            &result_for_refresh,
        );
    });
    refresh_storage(
        &table,
        &status,
        &sort_state,
        &destination_config,
        &destination_info,
        &destination_result,
    );
    page
}

fn refresh_storage(
    table: &gtk::Grid,
    status: &gtk::Label,
    sort_state: &Rc<RefCell<Option<(usize, bool)>>>,
    destination_config: &Rc<RefCell<Option<DestinationConfig>>>,
    destination_info: &gtk::Label,
    destination_result: &gtk::Label,
) {
    while let Some(child) = table.first_child() {
        table.remove(&child);
    }
    let sort = *sort_state.borrow();
    append_storage_header(
        table,
        sort,
        table,
        status,
        sort_state,
        destination_config,
        destination_info,
        destination_result,
    );

    let output = Command::new("lsblk")
        .args([
            "--json",
            "--bytes",
            "--output",
            "NAME,PATH,TYPE,FSTYPE,UUID,PARTTYPE,PARTLABEL,SIZE,FSAVAIL,FSUSE%,MOUNTPOINTS,MODEL",
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

    let validation = destination_config
        .borrow()
        .as_ref()
        .map(|config| validate_destination(config, &devices));
    update_destination_info(
        destination_info,
        destination_config.borrow().as_ref(),
        &devices,
        validation.as_deref(),
    );
    destination_result.set_text(
        validation
            .as_deref()
            .unwrap_or("No active backup destination is configured."),
    );

    if devices.is_empty() {
        status.set_text("No storage devices or partitions were found.");
        return;
    }

    if let Some((column, ascending)) = sort {
        devices.sort_by(|left, right| {
            let ordering = compare_device_values(left, right, column);
            let ordering = if ascending {
                ordering
            } else {
                ordering.reverse()
            };
            ordering.then_with(|| compare_device_values(left, right, 0))
        });
    }

    let count = devices.len();
    for (index, device) in devices.iter().enumerate() {
        append_device_row(
            table,
            device,
            index as i32 + 1,
            destination_config,
            destination_info,
            destination_result,
            status,
            sort_state,
        );
    }
    status.set_text(&format!(
        "Found {count} devices and partitions. Information is read-only."
    ));
}

fn destination_config_file() -> PathBuf {
    gtk::glib::user_config_dir()
        .join("cadence")
        .join("config.json")
}

fn destination_path_for(mountpoint: &str) -> PathBuf {
    Path::new(mountpoint).join("cadence-snap")
}

fn create_destination_config(mountpoint: &str, uuid: &str) -> Result<DestinationConfig, String> {
    if uuid.trim().is_empty() {
        return Err("The selected volume does not expose a filesystem UUID; its identity cannot be verified.".to_string());
    }
    let mount = Path::new(mountpoint);
    if !mount.is_absolute() || !mount.is_dir() {
        return Err(
            "The selected volume is no longer mounted at the expected directory.".to_string(),
        );
    }
    let mounted_uuid = Command::new("findmnt")
        .args(["--noheadings", "--output", "UUID", "--target", mountpoint])
        .output()
        .map_err(|error| format!("Could not verify the mounted filesystem identity: {error}"))?;
    if !mounted_uuid.status.success() {
        return Err(
            "Could not verify the filesystem mounted at the selected location.".to_string(),
        );
    }
    let mounted_uuid = String::from_utf8_lossy(&mounted_uuid.stdout)
        .trim()
        .to_string();
    if mounted_uuid != uuid {
        return Err("The filesystem mounted at this location changed. Refresh the device list and try again.".to_string());
    }

    let writable = Command::new("test")
        .args(["-w", mountpoint])
        .status()
        .map_err(|error| format!("Could not verify write access: {error}"))?;
    if !writable.success() {
        return Err("The selected volume is not writable by the current user.".to_string());
    }

    let path = destination_path_for(mountpoint);
    if path.exists() {
        if !path.is_dir() {
            return Err("A file already exists at the cadence-snap destination path.".to_string());
        }
        let writable = Command::new("test")
            .args(["-w", path.to_string_lossy().as_ref()])
            .status()
            .map_err(|error| format!("Could not verify destination access: {error}"))?;
        if !writable.success() {
            return Err(
                "The existing cadence-snap directory is not writable by the current user."
                    .to_string(),
            );
        }
    }

    Ok(DestinationConfig {
        mountpoint: mountpoint.to_string(),
        path: path.to_string_lossy().into_owned(),
        uuid: Some(uuid.to_string()),
    })
}

fn load_destination_config() -> Option<DestinationConfig> {
    let content = fs::read_to_string(destination_config_file()).ok()?;
    let value: Value = serde_json::from_str(&content).ok()?;
    destination_from_json(&value)
}

fn save_destination_config(config: &DestinationConfig) -> Result<(), String> {
    if Path::new(&config.path) != destination_path_for(&config.mountpoint) {
        return Err(
            "The destination must be cadence-snap at the root of the selected volume.".to_string(),
        );
    }
    let file = destination_config_file();
    let directory = file
        .parent()
        .ok_or_else(|| "Invalid configuration directory.".to_string())?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not create configuration directory: {error}"))?;
    let temporary = file.with_extension("json.tmp");
    let mut content = fs::read_to_string(&file)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    let backup = BackupConfig::from_json(&content);
    content["schema_version"] = serde_json::json!(1);
    content["destination"] = serde_json::json!({
        "mountpoint": config.mountpoint,
        "path": config.path,
        "uuid": config.uuid,
    });
    content["backup"] = backup.to_json();
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&content).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Could not write destination settings: {error}"))?;
    fs::rename(&temporary, &file)
        .map_err(|error| format!("Could not finalize destination settings: {error}"))?;
    Ok(())
}

fn update_destination_info(
    label: &gtk::Label,
    config: Option<&DestinationConfig>,
    devices: &[Value],
    validation: Option<&str>,
) {
    let Some(config) = config else {
        label.set_text(
            "Status: Not configured\nVolume: —\nMount point: —\nSnapshot directory: —\nAvailable space: —",
        );
        return;
    };

    let active = validation.is_some_and(|result| result.starts_with("Valid —"));
    let mounted_device = devices
        .iter()
        .find(|device| device_mountpoint(device).as_deref() == Some(config.mountpoint.as_str()));
    let verified_device = mounted_device.filter(|device| {
        config.uuid.as_deref().is_some_and(|expected_uuid| {
            json_text(device, "uuid").as_deref() == Some(expected_uuid)
        })
    });

    let volume = verified_device
        .and_then(|device| json_text(device, "path").or_else(|| json_text(device, "name")))
        .unwrap_or_else(|| match mounted_device {
            Some(_) => "Identity not verified".to_string(),
            None => "Unavailable".to_string(),
        });
    let available_space = verified_device
        .and_then(|device| device.get("fsavail").and_then(Value::as_u64))
        .map(|bytes| format!("{} GB", format_gigabytes(bytes)))
        .unwrap_or_else(|| "—".to_string());

    label.set_text(&format!(
        "Status: {}\nVolume: {}\nMount point: {}\nSnapshot directory: {}\nAvailable space: {}",
        if active { "Active" } else { "Unavailable" },
        volume,
        config.mountpoint,
        config.path,
        available_space,
    ));
}

fn validate_destination(config: &DestinationConfig, devices: &[Value]) -> String {
    let Some(expected_uuid) = config.uuid.as_deref() else {
        return "Identity not verified — select this volume again with Use".to_string();
    };

    let Some(device) = devices
        .iter()
        .find(|device| device_mountpoint(device).as_deref() == Some(config.mountpoint.as_str()))
    else {
        return "Volume unavailable — the configured mount point is not present".to_string();
    };

    let Some(current_uuid) = json_text(device, "uuid") else {
        return "Identity could not be verified — the mounted volume has no readable UUID"
            .to_string();
    };
    if current_uuid != expected_uuid {
        return "Identity mismatch — a different filesystem is mounted at this location"
            .to_string();
    }

    let assessment = destination_status(device);
    match assessment {
        "Read-only" => return "Read-only — the volume is not writable".to_string(),
        "No free space" => {
            return "Insufficient space — the filesystem reports no free space".to_string();
        }
        "Check free space" => {
            return "Free space unknown — capacity could not be verified".to_string();
        }
        "Mount not verified" | "Mount mode unknown" => {
            return "Mount access could not be verified".to_string();
        }
        "Potential device" => {}
        other => return format!("Volume is not eligible: {other}"),
    }

    if !Path::new(&config.mountpoint).is_dir() {
        return "Volume unavailable — the mount directory no longer exists".to_string();
    }

    let writable = Command::new("test")
        .args(["-w", &config.mountpoint])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !writable {
        return "Not writable — the current user cannot write to the volume".to_string();
    }

    let destination = Path::new(&config.path);
    if destination.exists() {
        if !destination.is_dir() {
            return "Invalid destination — cadence-snap exists but is not a directory".to_string();
        }
        let writable = Command::new("test")
            .args(["-w", &config.path])
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !writable {
            return "Not writable — the cadence-snap directory cannot be written".to_string();
        }
    }

    "Valid — volume identity, mount, permissions and available space verified".to_string()
}

fn update_destination_error(label: &gtk::Label, error: &str) {
    label.set_text(error);
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

fn storage_column_alignment(_column: usize) -> f32 {
    // Every heading and every value starts at the same left edge.
    0.0
}

fn storage_cell(column: usize) -> gtk::Box {
    let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    cell.set_size_request(STORAGE_COLUMNS[column].1, -1);
    cell.set_hexpand(true);
    cell.set_halign(gtk::Align::Fill);
    cell
}

fn append_storage_header(
    table: &gtk::Grid,
    sort: Option<(usize, bool)>,
    table_for_sort: &gtk::Grid,
    status_for_sort: &gtk::Label,
    sort_state: &Rc<RefCell<Option<(usize, bool)>>>,
    destination_config: &Rc<RefCell<Option<DestinationConfig>>>,
    destination_info: &gtk::Label,
    destination_result: &gtk::Label,
) {
    for (column, (title, _)) in STORAGE_COLUMNS.iter().enumerate() {
        let label = if let Some((sorted_column, ascending)) = sort {
            if sorted_column == column {
                format!("{title} {}", if ascending { "↑" } else { "↓" })
            } else {
                (*title).to_string()
            }
        } else {
            (*title).to_string()
        };

        let cell = storage_cell(column);
        cell.set_margin_top(8);
        cell.set_margin_bottom(8);

        let header_label = gtk::Label::new(Some(&label));
        header_label.add_css_class("heading");
        header_label.set_hexpand(true);
        header_label.set_halign(gtk::Align::Fill);
        header_label.set_xalign(storage_column_alignment(column));
        header_label.set_tooltip_text(Some(&format!("Sort by {title}")));
        cell.append(&header_label);

        // Sorting is attached to the shared cell so the clickable header uses
        // exactly the same column width and alignment container as every data row.
        let table_weak = table_for_sort.downgrade();
        let status_weak = status_for_sort.downgrade();
        let destination_config = destination_config.clone();
        let destination_info = destination_info.clone();
        let destination_result = destination_result.clone();
        let sort_state = sort_state.clone();
        let gesture = gtk::GestureClick::new();
        gesture.connect_released(move |_, _, _, _| {
            if let (Some(table), Some(status)) = (table_weak.upgrade(), status_weak.upgrade()) {
                let next_sort = match *sort_state.borrow() {
                    Some((current_column, ascending)) if current_column == column => {
                        Some((column, !ascending))
                    }
                    _ => Some((column, true)),
                };
                *sort_state.borrow_mut() = next_sort;
                refresh_storage(
                    &table,
                    &status,
                    &sort_state,
                    &destination_config,
                    &destination_info,
                    &destination_result,
                );
            }
        });
        cell.add_controller(gesture);

        table.attach(&cell, column as i32, 0, 1, 1);
    }
}

fn compare_device_values(left: &Value, right: &Value, column: usize) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    match column {
        0 => compare_optional_text(
            json_text(left, "path").or_else(|| json_text(left, "name")),
            json_text(right, "path").or_else(|| json_text(right, "name")),
        ),
        1 => compare_optional_text(json_text(left, "type"), json_text(right, "type")),
        2 => compare_optional_text(json_text(left, "fstype"), json_text(right, "fstype")),
        3 => compare_optional_number(
            left.get("size").and_then(Value::as_u64),
            right.get("size").and_then(Value::as_u64),
        ),
        4 => compare_optional_number(
            left.get("fsavail").and_then(Value::as_u64),
            right.get("fsavail").and_then(Value::as_u64),
        ),
        5 => compare_optional_text(device_mountpoint(left), device_mountpoint(right)),
        6 => destination_status(left).cmp(destination_status(right)),
        _ => Ordering::Equal,
    }
}

fn compare_optional_text(left: Option<String>, right: Option<String>) -> std::cmp::Ordering {
    left.unwrap_or_default()
        .to_lowercase()
        .cmp(&right.unwrap_or_default().to_lowercase())
}

fn compare_optional_number(left: Option<u64>, right: Option<u64>) -> std::cmp::Ordering {
    left.unwrap_or(0).cmp(&right.unwrap_or(0))
}

fn append_device_row(
    table: &gtk::Grid,
    device: &Value,
    row_index: i32,
    destination_config: &Rc<RefCell<Option<DestinationConfig>>>,
    destination_info: &gtk::Label,
    destination_result: &gtk::Label,
    status: &gtk::Label,
    sort_state: &Rc<RefCell<Option<(usize, bool)>>>,
) {
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
    let assessment = destination_status(device);
    let selected_config = destination_config.borrow();
    let selected = mountpoint
        .as_deref()
        .map(|current| {
            selected_config
                .as_ref()
                .map(|config| config.mountpoint == current)
                .unwrap_or(false)
        })
        .unwrap_or(false);
    let destination = if selected {
        match selected_config
            .as_ref()
            .and_then(|config| config.uuid.as_deref())
        {
            None => "Identity not verified",
            Some(expected_uuid) => match json_text(device, "uuid").as_deref() {
                Some(current_uuid) if current_uuid == expected_uuid => "Selected",
                Some(_) => "Identity mismatch",
                None => "Identity unavailable",
            },
        }
    } else {
        assessment
    };
    drop(selected_config);

    let text_values = [path, kind, filesystem, size, available];

    for (column, value) in text_values.into_iter().enumerate() {
        let cell = storage_cell(column);
        cell.set_margin_top(8);
        cell.set_margin_bottom(8);

        let label = gtk::Label::new(Some(&value));
        label.set_xalign(storage_column_alignment(column));
        label.set_halign(gtk::Align::Fill);
        label.set_hexpand(true);
        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        if column == 3 || column == 4 {
            label.add_css_class("numeric");
        }
        cell.append(&label);
        table.attach(&cell, column as i32, row_index, 1, 1);
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
    let mount_cell = storage_cell(5);
    mount_cell.set_margin_top(4);
    mount_cell.set_margin_bottom(4);
    mount_button.set_halign(gtk::Align::Start);
    mount_button.set_tooltip_text(Some(mountpoint_text));
    mount_cell.append(&mount_button);
    table.attach(&mount_cell, 5, row_index, 1, 1);

    let explanation = destination_explanation(destination);
    let (status_icon, _status_css) = destination_icon(destination);
    let status_button = make_detail_button(
        status_icon,
        "Show destination assessment details",
        destination,
        explanation,
    );
    let destination_cell = storage_cell(6);
    destination_cell.set_margin_top(4);
    destination_cell.set_margin_bottom(4);
    status_button.set_halign(gtk::Align::Start);
    destination_cell.append(&status_button);
    let eligible = assessment == "Potential device";
    if eligible && destination != "Selected" {
        if let (Some(mountpoint), Some(uuid)) = (mountpoint, json_text(device, "uuid")) {
            let select = gtk::Button::with_label("Use");
            select.add_css_class("flat");
            select.set_tooltip_text(Some("Use this volume for Cadence snapshots"));
            let config_state = destination_config.clone();
            let info_label = destination_info.clone();
            let result_label = destination_result.clone();
            let table_for_selection = table.clone();
            let status_for_selection = status.clone();
            let sort_for_selection = sort_state.clone();
            let mountpoint_for_click = mountpoint.clone();
            let uuid_for_click = uuid.clone();
            select.connect_clicked(move |_| {
                match create_destination_config(&mountpoint_for_click, &uuid_for_click) {
                    Ok(config) => match save_destination_config(&config) {
                        Ok(()) => {
                            *config_state.borrow_mut() = Some(config.clone());
                            result_label.set_text("Destination successfully set as the active backup destination.");
                            refresh_storage(
                                &table_for_selection,
                                &status_for_selection,
                                &sort_for_selection,
                                &config_state,
                                &info_label,
                                &result_label,
                            );
                        }
                        Err(error) => update_destination_error(
                            &result_label,
                            &format!("Could not save destination settings: {error}"),
                        ),
                    },
                    Err(error) => update_destination_error(&result_label, &error),
                }
            });
            destination_cell.append(&select);
        }
    }
    table.attach(&destination_cell, 6, row_index, 1, 1);
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
        "Selected" => ("emblem-default-symbolic", "success"),
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
        "Selected" => {
            "This volume is the currently selected destination for Cadence snapshots. Its current validation result is shown in the Backup destination panel."
        }
        "Identity not verified" => {
            "This saved destination predates filesystem identity tracking. Select Use again to record and verify its filesystem UUID."
        }
        "Identity mismatch" => {
            "A different filesystem is currently mounted at the saved mount point. Select Use to explicitly associate this mounted filesystem, or reconnect the original volume."
        }
        "Identity unavailable" => {
            "The mounted filesystem does not expose a readable UUID, so Cadence cannot safely confirm its identity."
        }
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
        "Mount mode unknown" => "The mount options do not clearly indicate read-write access.",
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


fn load_backup_config() -> BackupConfig {
    fs::read_to_string(destination_config_file())
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .map(|value| BackupConfig::from_json(&value))
        .unwrap_or_default()
}

fn save_backup_config(config: &BackupConfig) -> Result<(), String> {
    let file = destination_config_file();
    let directory = file
        .parent()
        .ok_or_else(|| "Invalid configuration directory.".to_string())?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not create configuration directory: {error}"))?;
    let temporary = file.with_extension("json.tmp");
    let mut content = fs::read_to_string(&file)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    content["schema_version"] = serde_json::json!(1);
    content["backup"] = config.to_json();
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&content).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Could not write backup settings: {error}"))?;
    fs::rename(&temporary, &file)
        .map_err(|error| format!("Could not finalize backup settings: {error}"))?;
    Ok(())
}

fn make_settings_page() -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 18);
    page.set_hexpand(true);
    page.set_vexpand(true);
    page.set_margin_top(32);
    page.set_margin_bottom(28);
    page.set_margin_start(32);
    page.set_margin_end(32);

    let heading = gtk::Label::new(Some("Backup settings"));
    heading.add_css_class("title-1");
    heading.set_halign(gtk::Align::Start);
    let subtitle = gtk::Label::new(Some(
        "Choose which parts of this computer Cadence will include in future backups.",
    ));
    subtitle.add_css_class("dim-label");
    subtitle.set_halign(gtk::Align::Start);
    subtitle.set_wrap(true);

    let config = Rc::new(RefCell::new(load_backup_config()));
    let status = gtk::Label::new(Some("Settings are saved automatically."));
    status.add_css_class("dim-label");
    status.set_halign(gtk::Align::Start);
    status.set_xalign(0.0);
    status.set_wrap(true);

    let content_card = gtk::Frame::new(None);
    content_card.add_css_class("card");
    content_card.set_hexpand(true);
    let content_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content_box.set_margin_top(16);
    content_box.set_margin_bottom(16);
    content_box.set_margin_start(18);
    content_box.set_margin_end(18);
    let content_heading = gtk::Label::new(Some("Backup content"));
    content_heading.add_css_class("heading");
    content_heading.set_halign(gtk::Align::Start);
    let system_check = gtk::CheckButton::with_label("System files");
    system_check.set_active(config.borrow().system_enabled);
    let user_data_check = gtk::CheckButton::with_label("User data");
    user_data_check.set_active(config.borrow().user_data_enabled);
    content_box.append(&content_heading);
    content_box.append(&system_check);
    content_box.append(&user_data_check);
    content_card.set_child(Some(&content_box));

    let user_card = gtk::Frame::new(None);
    user_card.add_css_class("card");
    user_card.set_hexpand(true);
    let user_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    user_box.set_margin_top(16);
    user_box.set_margin_bottom(16);
    user_box.set_margin_start(18);
    user_box.set_margin_end(18);
    let user_heading = gtk::Label::new(Some("User accounts"));
    user_heading.add_css_class("heading");
    user_heading.set_halign(gtk::Align::Start);
    let user_description = gtk::Label::new(Some(
        "All detected interactive local accounts are included by default. You can choose accounts individually.",
    ));
    user_description.add_css_class("dim-label");
    user_description.set_halign(gtk::Align::Start);
    user_description.set_wrap(true);
    let accounts_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    match discover_local_users() {
        Ok(users) if users.is_empty() => {
            let empty = gtk::Label::new(Some("No eligible local user accounts were detected."));
            empty.add_css_class("dim-label");
            empty.set_halign(gtk::Align::Start);
            accounts_box.append(&empty);
        }
        Ok(users) => {
            let all_homes: Vec<String> = users
                .iter()
                .map(|user| user.home.to_string_lossy().to_string())
                .collect();
            for user in users {
                let home = user.home.to_string_lossy().to_string();
                let selected = {
                    let current = config.borrow();
                    current.user_selection_mode == "all"
                        || current.selected_users.iter().any(|path| path == &home)
                };
                let check = gtk::CheckButton::with_label(&format!(
                    "{}  —  {}",
                    user.username, home
                ));
                check.set_active(selected);
                let config_for_user = config.clone();
                let status_for_user = status.clone();
                let all_homes_for_user = all_homes.clone();
                check.connect_toggled(move |button| {
                    let mut current = config_for_user.borrow_mut();
                    if current.user_selection_mode == "all" {
                        current.selected_users = all_homes_for_user.clone();
                        current.user_selection_mode = "selected".to_string();
                    }
                    if button.is_active() {
                        if !current.selected_users.iter().any(|path| path == &home) {
                            current.selected_users.push(home.clone());
                        }
                    } else {
                        current.selected_users.retain(|path| path != &home);
                    }
                    current.selected_users.sort();
                    current.selected_users.dedup();
                    match save_backup_config(&current) {
                        Ok(()) => status_for_user.set_text("Backup settings saved."),
                        Err(error) => status_for_user.set_text(&format!("Could not save settings: {error}")),
                    }
                });
                accounts_box.append(&check);
            }
        }
        Err(error) => {
            let failure = gtk::Label::new(Some(&format!("Could not detect local users: {error}")));
            failure.add_css_class("error");
            failure.set_halign(gtk::Align::Start);
            failure.set_wrap(true);
            accounts_box.append(&failure);
        }
    }

    user_box.append(&user_heading);
    user_box.append(&user_description);
    user_box.append(&accounts_box);
    user_card.set_child(Some(&user_box));

    let mode_card = gtk::Frame::new(None);
    mode_card.add_css_class("card");
    mode_card.set_hexpand(true);
    let mode_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    mode_box.set_margin_top(16);
    mode_box.set_margin_bottom(16);
    mode_box.set_margin_start(18);
    mode_box.set_margin_end(18);
    let mode_heading = gtk::Label::new(Some("User data inclusion"));
    mode_heading.add_css_class("heading");
    mode_heading.set_halign(gtk::Align::Start);
    let mode_description = gtk::Label::new(Some(
        "Choose whether to include every file or only hidden files in the selected home directories.",
    ));
    mode_description.add_css_class("dim-label");
    mode_description.set_halign(gtk::Align::Start);
    mode_description.set_wrap(true);
    let mode = gtk::ComboBoxText::new();
    mode.append(Some("all"), "All files");
    mode.append(Some("hidden_only"), "Hidden files only");
    mode.set_active_id(Some(&config.borrow().user_data_mode));
    mode_box.append(&mode_heading);
    mode_box.append(&mode_description);
    mode_box.append(&mode);
    mode_card.set_child(Some(&mode_box));

    let config_for_system = config.clone();
    let status_for_system = status.clone();
    system_check.connect_toggled(move |button| {
        let mut current = config_for_system.borrow_mut();
        current.system_enabled = button.is_active();
        match save_backup_config(&current) {
            Ok(()) => status_for_system.set_text("Backup settings saved."),
            Err(error) => status_for_system.set_text(&format!("Could not save settings: {error}")),
        }
    });

    let config_for_user_data = config.clone();
    let status_for_user_data = status.clone();
    let accounts_for_user_data = accounts_box.clone();
    user_data_check.connect_toggled(move |button| {
        let mut current = config_for_user_data.borrow_mut();
        current.user_data_enabled = button.is_active();
        accounts_for_user_data.set_sensitive(button.is_active());
        match save_backup_config(&current) {
            Ok(()) => status_for_user_data.set_text("Backup settings saved."),
            Err(error) => status_for_user_data.set_text(&format!("Could not save settings: {error}")),
        }
    });
    accounts_box.set_sensitive(config.borrow().user_data_enabled);

    let config_for_mode = config.clone();
    let status_for_mode = status.clone();
    mode.connect_changed(move |combo| {
        let Some(mode_id) = combo.active_id() else {
            return;
        };
        let mut current = config_for_mode.borrow_mut();
        current.user_data_mode = mode_id.to_string();
        match save_backup_config(&current) {
            Ok(()) => status_for_mode.set_text("Backup settings saved."),
            Err(error) => status_for_mode.set_text(&format!("Could not save settings: {error}")),
        }
    });

    page.append(&heading);
    page.append(&subtitle);
    page.append(&content_card);
    page.append(&user_card);
    page.append(&mode_card);
    page.append(&status);
    page
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
    use super::{BackupConfig, assess_destination, destination_from_json, destination_path_for};

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
    fn uid_min_is_read_from_login_defs_and_ignores_comments() {
        assert_eq!(
            super::configured_uid_min("# UID_MIN 500\nUID_MIN 1200\nUID_MAX 60000"),
            Some(1200)
        );
        assert_eq!(super::configured_uid_min("# UID_MIN 500"), None);
    }

    #[test]
    fn login_shells_ignore_comments_and_non_absolute_entries() {
        assert_eq!(
            super::parse_login_shells("# comment\n/bin/bash\ninvalid\n/usr/bin/zsh # zsh"),
            vec!["/bin/bash", "/usr/bin/zsh"]
        );
    }

    #[test]
    fn local_user_detection_filters_system_accounts_and_invalid_homes() {
        let passwd = concat!(
            "root:x:0:0:root:/root:/bin/bash\n",
            "daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin\n",
            "alice:x:1000:1000:Alice:/home/alice:/bin/bash\n",
            "bob:x:1001:1001:Bob:/data/bob:/usr/bin/zsh\n",
            "service:x:1500:1500:Service:/srv/service:/bin/false\n",
            "missing:x:1600:1600:Missing:/home/missing:/bin/bash\n",
            "relative:x:1700:1700:Relative:home/relative:/bin/bash\n",
            "malformed:x:not-a-uid:1800:Bad:/home/bad:/bin/bash\n"
        );
        let shells = vec!["/bin/bash".to_string(), "/usr/bin/zsh".to_string()];
        let users = super::parse_local_users(passwd, 1000, &shells, |home| {
            matches!(home.to_str(), Some("/home/alice" | "/data/bob"))
        });

        assert_eq!(users.len(), 2);
        assert_eq!(users[0].username, "alice");
        assert_eq!(users[0].uid, 1000);
        assert_eq!(users[0].home, std::path::PathBuf::from("/home/alice"));
        assert_eq!(users[1].username, "bob");
        assert_eq!(users[1].home, std::path::PathBuf::from("/data/bob"));
    }

    #[test]
    fn duplicate_local_user_records_are_not_returned_twice() {
        let passwd = concat!(
            "alice:x:1000:1000:Alice:/home/alice:/bin/bash\n",
            "alice:x:1000:1000:Alice:/home/alice:/bin/bash\n"
        );
        let users = super::parse_local_users(passwd, 1000, &["/bin/bash".to_string()], |_| true);
        assert_eq!(users.len(), 1);
    }

    #[test]
    fn backup_configuration_defaults_to_system_and_all_user_data() {
        let config = BackupConfig::default();
        assert!(config.system_enabled);
        assert!(config.user_data_enabled);
        assert_eq!(config.user_data_mode, "all");
        assert!(config.selected_users.is_empty());
    }

    #[test]
    fn legacy_destination_configuration_remains_readable() {
        let value = serde_json::json!({
            "mountpoint": "/media/backup",
            "path": "/media/backup/cadence-snap",
            "uuid": "test-uuid"
        });
        let config = destination_from_json(&value).unwrap();
        assert_eq!(config.mountpoint, "/media/backup");
        assert_eq!(config.uuid.as_deref(), Some("test-uuid"));
    }

    #[test]
    fn nested_destination_configuration_is_readable() {
        let value = serde_json::json!({
            "schema_version": 1,
            "destination": {
                "mountpoint": "/media/backup",
                "path": "/media/backup/cadence-snap",
                "uuid": "test-uuid"
            },
            "backup": BackupConfig::default().to_json()
        });
        let config = destination_from_json(&value).unwrap();
        assert_eq!(config.path, "/media/backup/cadence-snap");
    }

    #[test]
    fn invalid_backup_user_data_mode_falls_back_to_all() {
        let value = serde_json::json!({
            "backup": {
                "user_data": { "include_mode": "unknown" }
            }
        });
        assert_eq!(BackupConfig::from_json(&value).user_data_mode, "all");
    }

    #[test]
    fn snapshot_directory_is_at_the_volume_root() {
        assert_eq!(
            destination_path_for("/run/media/user/BACKUP").to_string_lossy(),
            "/run/media/user/BACKUP/cadence-snap"
        );
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
