use core::fmt;
use std::{
    env::args,
    ffi::CString,
    fs,
    path::{Path, PathBuf},
};

use crate::utils::{debug, libc_wrap, log, wrap};
use clap::{Args, Parser};
use libc::rename;

const PROGRAM: &str = "mv";

#[derive(Parser, Debug, Clone)]
#[command(
    version,
    about = "Move (and rename!) files and directories",
    author = "Alexander Hübner"
)]
struct Cli {
    #[clap(value_parser, required = true)]
    source: Vec<PathBuf>,
    #[clap(value_parser, required = true)]
    destination: PathBuf,
    // Done
    #[arg(long = "backup", help = "Make a backup of each file")]
    backup_choice: Option<Choice>,
    // Done
    #[arg(
        short = 'b',
        help = "Like --backup but doesnt take an argument (Default option is \"existing\")"
    )]
    backup: bool,
    // Done
    #[arg(long = "debug", help = "Debug, also activates verbose")]
    debug: bool,
    // TODO
    #[arg(
        long = "exchange",
        help = "Exchange source and destination (swap them)"
    )]
    exchange: bool,
    // TODO
    #[command(flatten)]
    destructive_actions: DestructiveActions,
    // Done
    #[arg(long = "no-copy", help = "Do not copy if renaming fails")]
    no_copy: bool,
    // Done
    #[arg(
        long = "strip-trailing-slashes",
        help = "Remove any trailing slashes from each SOURCE argument"
    )]
    strip_trailing_slashes: bool,
    // Done
    #[arg(
        short = 'S',
        long = "suffix",
        help = "Specify a backup suffix (Text appended to the end of a backup filename)"
    )]
    suffix: Option<String>,
    // Done
    #[arg(
        short = 't',
        long = "target-directory",
        help = "Treat destination as a directory"
    )]
    target_directory: bool,
    // Done
    #[arg(
        short = 'T',
        long = "no-target-directory",
        help = "Treat destination as a normal file"
    )]
    no_target_directory: bool,
    // TODO
    #[arg(long = "update", help = "Control which existing files are updated")]
    update: Option<Update>,
    // Done
    #[arg(short = 'v', long = "verbose", help = "Explain whats being done")]
    verbose: bool,
}

#[derive(Args, Clone, Copy, Debug)]
#[group(required = false, multiple = false)]
struct DestructiveActions {
    // TODO
    #[arg(
        short = 'f',
        long = "force",
        help = "Do not prompt before destructive actions"
    )]
    force: bool,
    // TODO
    #[arg(
        short = 'i',
        long = "interactive",
        help = "Prompt before destructive actions, opposite of force"
    )]
    interactive: bool,
    // TODO
    #[arg(
        short = 'n',
        long = "no-clobber",
        help = "Never do any destructive actions"
    )]
    no_clobber: bool,
}

#[derive(clap::ValueEnum, Clone, Debug, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Choice {
    /// Never make backups, even if --backup is given
    None,
    /// Alias of none
    Off,
    /// Make numbered backups
    Numbered,
    /// Alias of Numbered
    T,
    /// Make numbered backups if existing, otherwise simple backup
    Existing,
    /// Alias of existing
    Nil,
    /// Always make simple backups
    Simple,
    /// Alias of simple
    Never,
}

impl fmt::Display for Choice {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Choice::Existing | Choice::Nil => write!(f, "Existing/Nil"),
            Choice::None | Choice::Off => write!(f, "None/Off"),
            Choice::Numbered | Choice::T => write!(f, "Numbered/T"),
            Choice::Simple | Choice::Never => write!(f, "Simple/Never"),
        }
    }
}

#[derive(clap::ValueEnum, Clone, Debug, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
enum Update {
    /// Every file in destination is replaced
    #[default]
    All,
    /// No destination files are replaced, wont induce a failure
    None,
    /// Like none, but will induce a failure
    #[clap(name = "none-fail")]
    Nonefail,
    /// Destination files are replaced if they are older than source
    Older,
}

pub fn main() {
    let cli: Cli;
    // skip first arg if it happens to be "blutils"
    if args().collect::<Vec<String>>()[0]
        .split("/")
        .last()
        .unwrap()
        == "blutils"
    {
        cli = Cli::parse_from(args().skip(1));
    } else {
        cli = Cli::parse();
    };
    for p in &cli.source {
        log(cli.verbose || cli.debug, format!("Moving {}", p.display()));
        backup(&cli, p);
        mv(&cli, p);
    }
}

fn backup(cli: &Cli, p: &PathBuf) {
    // Checking for options and if the file exists
    if (!cli.backup && !cli.backup_choice.is_some()) || cli.destination.try_exists().is_err() {
        return;
    };

    let suffix = cli.suffix.clone().unwrap_or(String::from("~"));
    let mut backup_path = format!("{}{}", cli.destination.display(), suffix);
    let choice = cli.backup_choice.unwrap_or(Choice::Existing);

    log(
        cli.verbose || cli.debug,
        format!("Starting backup with choice {}", choice),
    );

    if choice == Choice::Nil || choice == Choice::Existing {
        if !Path::new(&backup_path).exists() {
            _ = wrap(fs::copy(p, backup_path), PROGRAM);
        } else {
            let mut i = 0;
            loop {
                backup_path = format!("{}{}{}", cli.destination.display(), suffix, i);
                if !Path::new(&backup_path).exists() {
                    _ = wrap(fs::copy(p, backup_path), PROGRAM);
                    log(cli.verbose || cli.debug, "Backup successful");
                    break;
                }
                i = i + 1;
            }
        }
    } else if choice == Choice::Numbered || choice == Choice::T {
        let mut i = 0;
        loop {
            backup_path = format!("{}{}{}", cli.destination.display(), suffix, i);
            if !Path::new(&backup_path).exists() {
                _ = wrap(fs::copy(p, backup_path), PROGRAM);
                log(cli.verbose || cli.debug, "Backup successful");
                break;
            }
            i = i + 1;
        }
    } else if choice == Choice::Simple || choice == Choice::Never {
        _ = wrap(fs::copy(p, backup_path), PROGRAM);
        log(cli.verbose || cli.debug, "Backup successful");
    }
}

fn mv(cli: &Cli, p: &PathBuf) {
    let source: CString;
    // If option is enabled, remove trailing slashes from source
    //
    // This also applies to no_target_directory
    if cli.strip_trailing_slashes || cli.no_target_directory {
        // Copy into a string since we need string manipulation for this!
        let mut source_copy = p.to_str().to_owned().unwrap().to_string();
        while source_copy.ends_with("/") {
            // Discard the result, we dont really care about it ¯\_(ツ)_/¯
            _ = source_copy.pop()
        }
        // When it doesnt end with a slash the loop ends and we create a CString from our new
        // string
        source = CString::new(source_copy).unwrap();
    } else if cli.target_directory {
        let mut source_copy = p.to_str().to_owned().unwrap().to_string();
        if !source_copy.ends_with("/") {
            source_copy.push('/');
        };
        source = CString::new(source_copy).unwrap();
    } else {
        source = CString::new(p.to_str().unwrap()).unwrap();
    };
    let dest = CString::new(cli.destination.to_str().unwrap()).unwrap();
    

    debug(
        cli.debug,
        format!(
            "Debug: Source: {}, Destination: {}",
            &source.to_str().unwrap(),
            &dest.to_str().unwrap()
        ),
    );
    debug(cli.debug, "Entering unsafe statement");


    unsafe {
        let rename_result = libc_wrap(rename(source.as_ptr(), dest.as_ptr()));
        if rename_result.is_err() {
            if !cli.no_copy {
                log(
                    cli.verbose || cli.debug,
                    "Renaming failed, copying instead!",
                );
                wrap(fs::copy(p, cli.destination.clone()), PROGRAM);
                log(cli.verbose || cli.debug, "Copying was successful!");
            } else {
                wrap(rename_result, PROGRAM);
            }
        }
        debug(cli.debug, "Exiting unsafe statement");
    };
}
