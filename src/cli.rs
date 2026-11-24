use std::time::Duration;

use clap::Parser;
use clap::builder::styling::{AnsiColor, Styles};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "monitor",
    version,
    about = "A modern watch: run a command periodically and view its output, with a time machine, diff highlighting and search.",
    long_about = None,
    styles = Styles::styled()
        .header(AnsiColor::Yellow.on_default())
        .usage(AnsiColor::Yellow.on_default())
        .literal(AnsiColor::Green.on_default()),
)]
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

    /// Start with line wrapping enabled
    #[arg(short = 'w', long)]
    pub wrap: bool,

    /// Don't capture the mouse: drag uses the terminal's own selection, and the wheel
    /// scrolls only where the terminal supports alternate scroll mode
    #[arg(long)]
    pub no_mouse: bool,

    /// Run the command directly instead of through a shell
    #[arg(short = 'x', long)]
    pub exec: bool,

    /// Shell used to run the command
    #[arg(short = 's', long, env = "MONITOR_SHELL", default_value = "sh")]
    pub shell: String,

    /// Extra options passed to the shell before `-c`, e.g. "-i"
    #[arg(
        long,
        value_name = "OPTIONS",
        allow_hyphen_values = true,
        default_value = ""
    )]
    pub shell_options: String,

    /// Exit when the command exits with a non-zero status
    #[arg(short = 'e', long)]
    pub errexit: bool,

    /// Exit when the output changes
    #[arg(short = 'g', long)]
    pub chgexit: bool,

    /// Record every run in the history, not only runs whose output or exit status changed
    #[arg(short = 'k', long)]
    pub keep_unchanged: bool,

    /// Maximum number of results kept in the history
    #[arg(long, value_name = "N", default_value_t = 1000, value_parser = clap::value_parser!(u64).range(1..))]
    pub max_history: u64,

    /// Maximum memory used by stored results, e.g. `64M`, `1G`; oldest results are dropped first.
    /// Unchanged results share memory with the previous one.
    #[arg(long, value_name = "SIZE", default_value = "128M", value_parser = parse_size)]
    pub max_history_size: usize,

    /// Command to run
    #[arg(required = true, num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<String>,
}

fn parse_interval(s: &str) -> Result<Duration, String> {
    let d = match s.parse::<f64>() {
        Ok(secs) if secs.is_finite() && secs >= 0.0 => {
            Duration::try_from_secs_f64(secs).map_err(|_| "interval is too large".to_string())?
        }
        Ok(_) => return Err("interval must be a non-negative number".into()),
        Err(_) => humantime::parse_duration(s).map_err(|e| e.to_string())?,
    };
    Ok(d.max(Duration::from_millis(10)))
}

fn parse_size(s: &str) -> Result<usize, String> {
    let s = s.trim();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let num: f64 = num.parse().map_err(|_| format!("invalid size `{s}`"))?;
    let mult = match unit
        .trim()
        .to_ascii_uppercase()
        .trim_end_matches("IB")
        .trim_end_matches('B')
    {
        "" => 1u64,
        "K" => 1 << 10,
        "M" => 1 << 20,
        "G" => 1 << 30,
        _ => return Err(format!("invalid size unit in `{s}` (use K, M or G)")),
    };
    Ok((num * mult as f64) as usize)
}

pub fn format_bytes(n: usize) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1}G", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1}M", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.0}K", n as f64 / 1024.0),
        n => format!("{n}B"),
    }
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
        assert_eq!(parse_size("128M").unwrap(), 128 << 20);
        assert_eq!(parse_size("1.5GiB").unwrap(), 3 << 29);
        assert_eq!(parse_size("4096").unwrap(), 4096);
        assert!(parse_size("12X").is_err());
    }

    #[test]
    fn trailing_command() {
        let cli = Cli::try_parse_from(["monitor", "-n", "1", "-d", "ls", "-l", "--all"]).unwrap();
        assert_eq!(cli.command, vec!["ls", "-l", "--all"]);
        assert!(cli.differences);
    }
}
