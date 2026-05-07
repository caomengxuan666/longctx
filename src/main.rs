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
        /// Seed for deterministic generation
        #[arg(long)]
        seed: Option<u64>,
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
        /// Output file for HTML or JSON summary
        #[arg(long)]
        out: Option<String>,
        /// Print a machine-readable JSON summary instead of HTML
        #[arg(long)]
        json: bool,
    },
    /// Compare two results JSONL files
    Compare {
        /// Baseline results JSONL file
        baseline: String,
        /// Candidate results JSONL file
        candidate: String,
        /// Print JSON instead of human-readable text
        #[arg(long)]
        json: bool,
        /// Write a standalone HTML comparison report
        #[arg(long)]
        html_out: Option<String>,
        /// Fail the command when any regression is detected
        #[arg(long)]
        fail_on_regression: bool,
    },
    /// Validate benchmark directory contents before running
    Validate {
        /// Benchmark directory containing config.toml and test cases
        bench_dir: String,
        /// Skip environment-variable checks for provider API keys
        #[arg(long)]
        skip_api_key_check: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Generate {
            suite,
            tokens,
            seed,
            out,
        } => {
            longctx::generator::generate(&suite, tokens, seed, &out)?;
        }
        Commands::Run { bench_dir } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(longctx::runner::run_benchmarks(&bench_dir))?;
        }
        Commands::Report { results, out, json } => {
            if json {
                let summary = longctx::report::generate_summary(&results)?;
                let json_summary = summary.to_json()?;
                if let Some(out) = out {
                    std::fs::write(&out, json_summary)?;
                } else {
                    println!("{json_summary}");
                }
            } else {
                let out = out.unwrap_or_else(|| "report.html".to_string());
                longctx::report::generate_html(&results, &out)?;
            }
        }
        Commands::Compare {
            baseline,
            candidate,
            json,
            html_out,
            fail_on_regression,
        } => {
            let summary = longctx::compare::compare_results(&baseline, &candidate)?;
            if let Some(html_out) = html_out {
                longctx::compare::write_comparison_html(&summary, &html_out)?;
            }
            if json {
                println!("{}", summary.to_json()?);
            } else {
                print!("{}", summary.render());
            }
            if fail_on_regression && !summary.regressed_ids.is_empty() {
                anyhow::bail!(
                    "{} regressions detected: {}",
                    summary.regressed_ids.len(),
                    summary.regressed_ids.join(", ")
                );
            }
        }
        Commands::Validate {
            bench_dir,
            skip_api_key_check,
        } => {
            let summary =
                longctx::validator::validate_benchmark_dir(&bench_dir, !skip_api_key_check)?;
            println!(
                "validated {} tests with model {}",
                summary.test_count, summary.config_model
            );
        }
    }
    Ok(())
}
