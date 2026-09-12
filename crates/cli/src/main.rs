use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "voxedit-cli")]
#[command(version)]
#[command(about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate test signals
    Gen,
    /// Analyze audio
    Analyze,
    /// Render audio with rack
    Render,
    /// Benchmark DSP modules
    Bench,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Gen) => {
            println!("not implemented yet");
        }
        Some(Commands::Analyze) => {
            println!("not implemented yet");
        }
        Some(Commands::Render) => {
            println!("not implemented yet");
        }
        Some(Commands::Bench) => {
            println!("not implemented yet");
        }
        None => {
            println!("no command specified");
        }
    }
}
