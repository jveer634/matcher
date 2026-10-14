use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt;

/// Formats for structured tracing logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-friendly, colorized ANSI format with caller location for terminal development.
    Pretty,
    /// Structured JSON format with caller location for cloud log aggregators (Datadog, Loki, CloudWatch).
    Json,
}

impl Default for LogFormat {
    fn default() -> Self {
        Self::Pretty
    }
}

/// Initializes the global tracing subscriber with default non-debug settings.
pub fn init_tracing() {
    init_tracing_with_config(false, None);
}

/// Initializes tracing subscriber with explicit format choice.
pub fn init_tracing_with_format(format: LogFormat) {
    init_tracing_with_config(false, Some(format));
}

/// Initializes tracing subscriber respecting `is_debug` from `config.toml` and environment overrides.
///
/// - If `is_debug` is true, enables `debug` logging for matcher domain events while suppressing framework noise.
/// - If `RUST_LOG` is set in the environment, it takes precedence.
/// - Caller location (`target`, `file`, and `line`) is included across all log messages.
/// - Output format is determined by `LOG_FORMAT` ("json" or "pretty", defaulting to `pretty`).
pub fn init_tracing_with_config(is_debug: bool, format_override: Option<LogFormat>) {
    let format = format_override.unwrap_or_else(|| {
        match std::env::var("LOG_FORMAT").unwrap_or_default().to_lowercase().as_str() {
            "json" => LogFormat::Json,
            _ => LogFormat::Pretty,
        }
    });

    let default_filter_str = if is_debug {
        "debug,h2=info,hyper=info,tower=info"
    } else {
        "info"
    };

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter_str));

    let builder = fmt::Subscriber::builder().with_env_filter(filter);

    match format {
        LogFormat::Json => {
            let _ = builder
                .json()
                .flatten_event(true)
                .with_target(true)
                .with_file(true)
                .with_line_number(true)
                .with_current_span(true)
                .with_span_list(false)
                .try_init();
        }
        LogFormat::Pretty => {
            let _ = builder
                .with_target(true)
                .with_file(true)
                .with_line_number(true)
                .with_thread_ids(false)
                .compact()
                .try_init();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_tracing_idempotent() {
        init_tracing_with_config(false, Some(LogFormat::Pretty));
        init_tracing_with_config(true, Some(LogFormat::Json));
    }
}
