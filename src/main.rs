use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "longctx", version, about = "Long-context benchmark suite")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate synthetic benchmark suites
    Generate {
        /// Suite type: needle, multi-needle, conflict
        suite: String,
        /// Number of tokens for the context
        #[arg(long, default_value = "100000")]
        tokens: u64,
        /// Output directory
        #[arg(long, default_value = "./bench")]
        out: String,
    },
    /// Run benchmarks against an API
    Run {
        /// Benchmark directory containing config.toml and test cases
        bench_dir: String,
    },
    /// Generate HTML report from results
    Report {
        /// Results JSONL file
        results: String,
        /// Output HTML file
        #[arg(long, default_value = "report.html")]
        out: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Generate { suite, tokens, out } => {
            longctx::generator::generate(&suite, tokens, &out)?;
        }
        Commands::Run { bench_dir } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(longctx::runner::run_benchmarks(&bench_dir))?;
        }
        Commands::Report { results, out } => {
            longctx::report::generate_html(&results, &out)?;
        }
    }
    Ok(())
}
