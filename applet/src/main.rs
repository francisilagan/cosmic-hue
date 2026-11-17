mod app;
mod backend;
mod model;

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<app::HueApplet>(())
}
