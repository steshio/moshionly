//! The `moshionly` command.
//!
//! - `moshionly [file.moshi]` opens the editor, with that project.
//! - `moshionly mcp [--save] [file.moshi]` serves MCP on standard input and
//!   output, for agents without a window.
//! - `moshionly export <file.moshi> --out <path> [options]` exports without
//!   a window.
//!
//! `moshionly --help` and each command's `--help` say more.

use moshionly_app::{
    APP_NAME,
    app::{MoshionlyApp, native_options},
    cli::{self, Command, EXIT_OK, EXIT_USAGE},
};

fn main() {
    let command = match cli::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("moshionly: {error}\n\n{}", error.usage);
            std::process::exit(EXIT_USAGE);
        }
    };
    let code = match command {
        Command::Help(text) => {
            print!("{text}");
            EXIT_OK
        }
        Command::Version => {
            println!("moshionly {}", env!("CARGO_PKG_VERSION"));
            EXIT_OK
        }
        Command::Mcp(args) => cli::run_mcp(&args),
        Command::Export(args) => cli::run_export(&args),
        Command::Window(project) => {
            let opened = eframe::run_native(
                APP_NAME,
                native_options(),
                Box::new(|cc| Ok(Box::new(MoshionlyApp::new(cc, project)))),
            );
            match opened {
                Ok(()) => EXIT_OK,
                Err(error) => {
                    eprintln!("moshionly: the window could not open: {error}");
                    cli::EXIT_FAILED
                }
            }
        }
    };
    std::process::exit(code);
}
