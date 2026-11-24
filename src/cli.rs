use std::time::Duration;

use clap::Parser;

/// A modern `watch` command: periodically run a command and view its output
/// full-screen, with a time machine (browse past results), diff highlighting,
/// search, scrolling and suspend.
#[derive(Parser, Debug, Clone)]
#[command(name = "monitor", version, about, long_about = None)]
pub struct Cli {
    /// Interval between runs, e.g. `2`, `0.5`, `500ms`, `1m30s`
    #[arg(short = 'n', long, default_value = "2s", value_parser = parse_interval)]
    pub interval: Duration,

    /// Highlight differences from the previous run
    #[arg(short = 'd', long)]
    pub differences: bool,

    /// Ring the terminal bell when the output changes
    #[arg(short = 'b', long)]
    pub bell: bool,

    /// Precise mode: start a run every interval regardless of how long the command takes
    #[arg(short = 'p', long)]
    pub precise: bool,

    /// Hide the title bar
    #[arg(short = 't', long = "no-title")]
    pub no_title: bool,

    /// Don't wrap long lines (scroll horizontally instead)
    #[arg(short = 'w', long)]
    pub unfold: bool,

    /// Run the command directly instead of through a shell
    #[arg(short = 'x', long)]
    pub exec: bool,

    /// Shell used to run the command
    #[arg(short = 's', long, env = "MONITOR_SHELL", default_value = "sh")]
    pub shell: String,

    /// Extra options passed to the shell before `-c`, e.g. "-i"
    #[arg(long, allow_hyphen_values = true, default_value = "")]
    pub shell_options: String,

    /// Exit when the command exits with a non-zero status
    #[arg(short = 'e', long)]
    pub errexit: bool,

    /// Exit when the output changes
    #[arg(short = 'g', long)]
    pub chgexit: bool,

    /// Maximum number of results kept in the history
    #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(1..))]
    pub max_history: u64,

    /// Command to run
    #[arg(required = true, num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<String>,
}

fn parse_interval(s: &str) -> Result<Duration, String> {
    let d = match s.parse::<f64>() {
        Ok(secs) if secs.is_finite() && secs >= 0.0 => Duration::from_secs_f64(secs),
        Ok(_) => return Err("interval must be a non-negative number".into()),
        Err(_) => humantime::parse_duration(s).map_err(|e| e.to_string())?,
    };
    Ok(d.max(Duration::from_millis(10)))
}

impl Cli {
    pub fn command_display(&self) -> String {
        self.command.join(" ")
    }
}

pub fn format_interval(d: Duration) -> String {
    if d.subsec_millis() == 0 {
        humantime::format_duration(Duration::from_secs(d.as_secs())).to_string()
    } else {
        humantime::format_duration(Duration::from_millis(d.as_millis() as u64)).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals() {
        assert_eq!(parse_interval("2").unwrap(), Duration::from_secs(2));
        assert_eq!(parse_interval("0.5").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_interval("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_interval("1m30s").unwrap(), Duration::from_secs(90));
        assert!(parse_interval("abc").is_err());
        assert_eq!(format_interval(Duration::from_millis(1500)), "1s 500ms");
    }

    #[test]
    fn trailing_command() {
        let cli = Cli::try_parse_from(["monitor", "-n", "1", "-d", "ls", "-l", "--all"]).unwrap();
        assert_eq!(cli.command, vec!["ls", "-l", "--all"]);
        assert!(cli.differences);
    }
}
