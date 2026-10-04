use clap::Parser;
use gitu::{
    Res,
    cli::{self, Args, Commands},
    config,
    error::Error,
    term,
};
use log::LevelFilter;
use std::{backtrace::Backtrace, fmt::Display, panic, process, sync::Arc};

pub fn main() -> Res<()> {
    let args = Args::parse();
    if args.version {
        // Setting cargo_suffix enables falling back to Cargo.toml for version
        // `cargo install --locked gitu` would fail otherwise, as there's no git repo
        println!("gitu {}", git_version::git_version!(cargo_suffix = ""));
        return Ok(());
    }

    // Generating completions doesn't need a git repository or the terminal,
    // so handle it before any of that setup happens.
    if let Some(Commands::Completion { shell }) = args.command {
        cli::completions(shell, &mut std::io::stdout());
        return Ok(());
    }

    if args.log {
        simple_logging::log_to_file(gitu::LOG_FILE_NAME, LevelFilter::Debug)
            .map_err(Error::OpenLogFile)?;
    }

    let config = Arc::new(config::init_config(args.config.clone())?);
    let config_ref = config.clone();

    panic::set_hook(Box::new(move |panic_info| {
        print_err(term::backend().reset_term(&config));

        eprintln!("{}", panic_info);
        eprintln!("trace: \n{}", Backtrace::force_capture());
    }));

    log::debug!("Starting app");
    let result = gitu::run(config_ref, &args, &mut term::backend());

    // A sequence editor that hands nothing back exits non-zero, so git calls
    // the rebase off.
    process::exit(result?);
}

fn print_err<T, E: Display>(result: Result<T, E>) {
    match result {
        Ok(_) => (),
        Err(error) => eprintln!("Error: {}", error),
    };
}
