use std::io::Read as _;

use nix_cargo_unit::nix_gen::{NixGenConfig, NixGenerator};
use nix_cargo_unit::unit_graph;

#[derive(clap::Parser)]
#[command(name = "nix-cargo-unit")]
#[command(about = "Convert cargo unit-graph to Nix derivations")]
struct Cli {
    /// Output format: nix or json
    #[arg(short, long, default_value = "nix")]
    format: String,

    /// Workspace root path for source remapping
    #[arg(short, long, default_value = ".")]
    workspace_root: String,

    /// Enable content-addressed derivations (CA-derivations)
    #[arg(long)]
    content_addressed: bool,

    /// Enable cross-compilation mode (use hostRustToolchain for proc-macros)
    #[arg(long)]
    cross_compile: bool,

    /// Host platform triple (for proc-macros and build scripts in cross-compilation)
    #[arg(long)]
    host_platform: Option<String>,

    /// Target platform triple (for regular crates in cross-compilation)
    #[arg(long)]
    target_platform: Option<String>,

    /// Toolchain hash to include in identity computation (prevents stale CA outputs when rustc changes)
    #[arg(long)]
    toolchain_hash: Option<String>,
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    use clap::Parser as _;
    let cli = Cli::parse();

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;

    let graph: unit_graph::UnitGraph = serde_json::from_str(&input)?;

    match cli.format.as_str() {
        "nix" => {
            let mut config = NixGenConfig {
                workspace_root: cli.workspace_root,
                content_addressed: cli.content_addressed,
                toolchain_hash: cli.toolchain_hash,
                ..Default::default()
            };

            // Configure cross-compilation if enabled
            if cli.cross_compile {
                config.cross_compiling = true;
                config.host_platform = cli.host_platform;
                config.target_platform = cli.target_platform;
            }

            let generator = NixGenerator::new(config);
            let nix = generator.generate(&graph);
            println!("{nix}");
        }
        "json" => {
            println!("{}", serde_json::to_string_pretty(&graph)?);
        }
        other => {
            color_eyre::eyre::bail!("unknown format: {other}");
        }
    }

    Ok(())
}
