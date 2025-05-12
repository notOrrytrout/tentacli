pub fn depends_on() {
    println!("called depends_on");
}

pub fn conditional() {
    println!("called conditional");
}

#[cfg(feature = "ui")]
pub mod ui;

#[cfg(feature = "console")]
pub mod console;

#[cfg(feature = "wotlk_login")]
pub mod wotlk_login;

#[cfg(feature = "wotlk_realm")]
pub mod wotlk_realm;
