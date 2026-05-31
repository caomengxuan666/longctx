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
        /// Suite type: needle, multi-needle, conflict, multi-hop, order-dependent, position-sweep, hallucination
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
        /// Overwrite existing results and request log files
        #[arg(long)]
        force: bool,
        /// Validate config, select tests, and resolve auto routing without sending provider requests
        #[arg(long)]
        dry_run: bool,
        /// Run only tests whose ID or suite contains this text
        #[arg(long)]
        filter: Option<String>,
        /// Run at most this many selected tests
        #[arg(long)]
        limit: Option<usize>,
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
    /// Build or refresh the context index used by auto routing
    Index {
        /// Benchmark directory containing manifests and contexts
        bench_dir: String,
    },
    /// Automatically probe maximum usable context and optional capability suites
    ProbeContext {
        /// Probe output directory. Use --config or place config.toml here.
        out_dir: String,
        /// Config file to copy into each generated probe benchmark directory
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// Initial token count for the exponential probe
        #[arg(long, default_value_t = 8_000)]
        min_tokens: u64,
        /// Upper token count cap for the probe
        #[arg(long, default_value_t = 1_000_000)]
        max_tokens: u64,
        /// Stop binary search when failure and success are within this many tokens
        #[arg(long, default_value_t = 8_000)]
        resolution_tokens: u64,
        /// Seed for deterministic generated probe data
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Token count for the capability suite pass
        #[arg(long, default_value_t = 32_000)]
        capability_tokens: u64,
        /// Skip multi-suite capability probing after max-context probing
        #[arg(long)]
        skip_capabilities: bool,
        /// Print a machine-readable JSON summary
        #[arg(long)]
        json: bool,
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
        Commands::Run {
            bench_dir,
            force,
            dry_run,
            filter,
            limit,
        } => {
            let options = longctx::runner::RunOptions {
                force,
                dry_run,
                filter,
                limit,
            };
            if dry_run {
                let summary = longctx::runner::dry_run_benchmarks(&bench_dir, options)?;
                println!(
                    "dry run ok: selected {}/{} tests for model {}",
                    summary.selected_count, summary.total_count, summary.provider_model
                );
                println!(
                    "auto contexts: {}/{} routed",
                    summary.routed_auto_context_count, summary.auto_context_count
                );
                println!("suites: {}", summary.suites.join(", "));
            } else {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(longctx::runner::run_benchmarks_with_options(
                    &bench_dir, options,
                ))?;
            }
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
                let out = out.unwrap_or_else(|| default_report_output(&results));
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
        Commands::Index { bench_dir } => {
            let bench_path = std::path::Path::new(&bench_dir);
            let index = longctx::context_index::build_context_index(bench_path)?;
            longctx::context_index::write_context_index(bench_path, &index)?;
            println!("indexed {} contexts", index.contexts.len());
        }
        Commands::ProbeContext {
            out_dir,
            config,
            min_tokens,
            max_tokens,
            resolution_tokens,
            seed,
            capability_tokens,
            skip_capabilities,
            json,
        } => {
            let options = longctx::probe::ProbeOptions {
                min_tokens,
                max_tokens,
                resolution_tokens,
                seed,
                config_path: config,
                capability_tokens: (!skip_capabilities).then_some(capability_tokens),
            };
            let rt = tokio::runtime::Runtime::new()?;
            let summary = rt.block_on(longctx::probe::probe_context(&out_dir, options))?;
            if json {
                println!("{}", summary.to_json()?);
            } else {
                print!("{}", summary.render());
            }
        }
    }
    Ok(())
}

fn default_report_output(results: &str) -> String {
    let results_path = std::path::Path::new(results);
    results_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("reports")
        .join("report.html")
        .to_string_lossy()
        .into_owned()
}
