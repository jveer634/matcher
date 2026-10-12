use clap::Parser;
use std::path::PathBuf;

/// Matcher: High-Performance In-Memory FIFO Trading Matching Engine CLI arguments.
#[derive(Parser, Debug, Clone, PartialEq, Eq)]
#[command(
    name = "matcher",
    version,
    about = "High-Performance In-Memory Matching Engine Microservice"
)]
pub struct Cli {
    /// Path to custom configuration file (defaults to config.toml)
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Run the interactive in-memory sharded execution showcase demo
    #[arg(short, long)]
    pub demo: bool,

    /// Optional TCP port override for the gRPC server
    #[arg(short, long)]
    pub port: Option<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_default_args() {
        let args = ["matcher"];
        let cli = Cli::try_parse_from(args).unwrap();
        assert!(!cli.demo);
        assert_eq!(cli.config, None);
        assert_eq!(cli.port, None);
    }

    #[test]
    fn test_cli_demo_flag() {
        let args = ["matcher", "--demo"];
        let cli = Cli::try_parse_from(args).unwrap();
        assert!(cli.demo);

        let args_short = ["matcher", "-d"];
        let cli_short = Cli::try_parse_from(args_short).unwrap();
        assert!(cli_short.demo);
    }

    #[test]
    fn test_cli_config_and_port() {
        let args = ["matcher", "-c", "custom.toml", "-p", "8080"];
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(cli.config, Some(PathBuf::from("custom.toml")));
        assert_eq!(cli.port, Some(8080));
    }
}
