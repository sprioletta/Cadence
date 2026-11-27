use adw::prelude::*;

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
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(180)
        .build();

    stack.add_named(
        &make_page(
            "drive-harddisk-symbolic",
            "Welcome to Cadence",
            "Your backup destinations and snapshots will appear here.",
        ),
        Some("overview"),
    );

    stack.add_named(
        &make_page(
            "document-save-symbolic",
            "Snapshots",
            "Browse and manage your system snapshots.",
        ),
        Some("snapshots"),
    );

    stack.add_named(
        &make_page(
            "drive-removable-media-symbolic",
            "Storage",
            "Discover and inspect backup storage devices.",
        ),
        Some("storage"),
    );

    stack.add_named(
        &make_page(
            "preferences-system-symbolic",
            "Settings",
            "Configure Cadence preferences.",
        ),
        Some("settings"),
    );

    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 6);
    sidebar.set_width_request(220);
    sidebar.set_margin_top(12);
    sidebar.set_margin_bottom(12);
    sidebar.set_margin_start(12);
    sidebar.set_margin_end(12);

    add_nav_button(&sidebar, &stack, "Overview", "overview", "go-home-symbolic");
    add_nav_button(
        &sidebar,
        &stack,
        "Snapshots",
        "snapshots",
        "document-save-symbolic",
    );
    add_nav_button(
        &sidebar,
        &stack,
        "Storage",
        "storage",
        "drive-removable-media-symbolic",
    );
    add_nav_button(
        &sidebar,
        &stack,
        "Settings",
        "settings",
        "preferences-system-symbolic",
    );

    let separator = gtk::Separator::new(gtk::Orientation::Vertical);

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
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
    label: &str,
    page: &str,
    icon_name: &str,
) {
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(18);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_margin_top(6);
    row.set_margin_bottom(6);
    row.set_margin_start(8);
    row.set_margin_end(8);
    row.append(&icon);
    row.append(&gtk::Label::new(Some(label)));

    let button = gtk::Button::new();
    button.set_child(Some(&row));
    button.set_hexpand(true);
    button.set_halign(gtk::Align::Fill);

    let stack_clone = stack.clone();
    let page_name = page.to_string();
    button.connect_clicked(move |_| {
        stack_clone.set_visible_child_name(&page_name);
    });

    sidebar.append(&button);
}
