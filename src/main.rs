use adw::prelude::*;
use gtk::prelude::*;

fn main() {
    adw::Application::builder()
        .application_id("io.github.sprioletta.Cadence")
        .build()
        .run();
}

#[gtk::main]
fn build_ui(application: &adw::Application) {
    let header = adw::HeaderBar::builder()
        .title_widget(&gtk::Label::new(Some("Cadence")))
        .build();

    let status_page = adw::StatusPage::builder()
        .icon_name("drive-harddisk-symbolic")
        .title("Welcome to Cadence")
        .description("Your system snapshots and backup destinations will appear here.")
        .build();

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&status_page));

    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("Cadence")
        .default_width(900)
        .default_height(600)
        .content(&toolbar_view)
        .build();

    window.present();
}

