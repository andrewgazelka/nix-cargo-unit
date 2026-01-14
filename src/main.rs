use std::io::Read as _;

mod unit_graph;

#[derive(clap::Parser)]
#[command(name = "nix-cargo-unit")]
#[command(about = "Convert cargo unit-graph to Nix derivations")]
struct Cli {
    /// Output format: nix or json
    #[arg(short, long, default_value = "nix")]
    format: String,
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
            let nix = graph.to_nix();
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
