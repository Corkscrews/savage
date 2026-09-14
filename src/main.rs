#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod document;
mod platform;
mod render;

use std::path::PathBuf;
use std::process::ExitCode;

use winit::event_loop::EventLoop;

use crate::app::App;

/// Events injected from the OS "Open With" path (macOS Apple Events).
#[derive(Debug)]
pub enum UserEvent {
    Open(PathBuf),
}

fn main() -> ExitCode {
    match parse_cli() {
        Cli::Help => {
            print_help();
            ExitCode::SUCCESS
        }
        Cli::Version => {
            println!("savage {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Cli::BadOption(flag) => {
            eprintln!("savage: unknown option {flag}");
            print_help();
            ExitCode::from(2)
        }
        Cli::Run { path } => {
            if let Err(err) = run(path) {
                eprintln!("savage: {err}");
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
    }
}

fn run(path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    platform::install();

    let mut builder = EventLoop::<UserEvent>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder.with_activation_policy(ActivationPolicy::Regular);
    }
    let event_loop = builder.build()?;
    platform::bind_proxy(event_loop.create_proxy());

    let mut app = App::new(path);
    event_loop.run_app(&mut app)?;
    Ok(())
}

enum Cli {
    Help,
    Version,
    BadOption(String),
    Run { path: Option<PathBuf> },
}

fn parse_cli() -> Cli {
    let mut args = std::env::args_os().skip(1);
    match args.next() {
        None => Cli::Run { path: None },
        Some(arg) if arg == "-h" || arg == "--help" => Cli::Help,
        Some(arg) if arg == "-V" || arg == "--version" => Cli::Version,
        Some(arg) => {
            let text = arg.to_string_lossy();
            if text.starts_with('-') {
                Cli::BadOption(text.into_owned())
            } else {
                Cli::Run {
                    path: Some(PathBuf::from(arg)),
                }
            }
        }
    }
}

fn print_help() {
    print!(
        "\
savage {} — fast SVG viewer

Usage:
  savage [FILE]

Opens FILE if given. Without a file, waits for a dropped SVG or the
OS default-app open request (Finder / Explorer / xdg).

Keys:
  Esc, Space, q    Quit
",
        env!("CARGO_PKG_VERSION")
    );
}
