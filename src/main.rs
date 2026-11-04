mod output;

use std::process::ExitCode;
use std::time::Duration;

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};

use lrcli::broker;
use lrcli::commands::{self, Context};
use lrcli::error::Error;
use lrcli::paths::Paths;

const ABOUT_EXAMPLES: &str = "\
Examples:
  lrcli status                 Check that Lightroom Classic is reachable
  lrcli --json status          Same, as JSON for scripts and agents
  lrcli plugin install         Link the plugin into Lightroom Classic

How it works:
  The lrcli plugin runs inside Lightroom Classic and starts one long-lived
  broker process. This command talks only to that broker, over a Unix socket
  in ~/.lrcli (override the directory with LRCLI_HOME).

Output:
  Human-readable text by default. --json (or LRCLI_OUTPUT=json) prints one
  JSON object on stdout. Errors are {\"ok\":false,\"error\":CODE,\"message\":TEXT};
  in human mode they go to stderr as `error: TEXT`.

Exit codes:
  0  success
  1  the command failed (plugin replied ok:false, or a local operation failed)
  2  usage error
  3  Lightroom Classic is unreachable (no broker, connection refused or lost)
  4  protocol or authentication mismatch between lrcli and the plugin
  5  timed out waiting for a reply";

#[derive(Parser)]
#[command(name = "lrcli", version, about = "Control Adobe Lightroom Classic from the command line", after_help = ABOUT_EXAMPLES)]
struct Cli {
    /// Print machine-readable JSON (also enabled by LRCLI_OUTPUT=json)
    #[arg(long, global = true)]
    json: bool,

    /// Seconds to wait for each request to be answered
    #[arg(long, global = true, value_name = "SECS", default_value = "30", value_parser = parse_secs)]
    timeout: f64,

    /// Seconds to wait for the broker connection to open
    #[arg(long, global = true, value_name = "SECS", default_value = "2", value_parser = parse_secs)]
    connect_timeout: f64,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show whether Lightroom Classic is reachable and what it is running
    #[command(after_help = "\
Asks the broker for the plugin's hello and prints the Lightroom Classic
version, plugin version and protocol, the open catalog's path, and the broker's
version and pid. A successful status means every other command can work.

Examples:
  lrcli status
  lrcli --json status

JSON output:
  {\"ok\":true,\"protocol\":1,\"plugin\":{\"version\":\"0.1.0\"},
   \"lightroom\":{\"version\":\"...\"},\"catalog\":{\"path\":\"...\"},
   \"broker\":{\"version\":\"0.1.0\",\"pid\":123}}

Exit 3 means Lightroom Classic or its plugin is not running; exit 4 means the
plugin rejected this lrcli (version or token mismatch).")]
    Status,

    /// Install, remove, or locate the Lightroom Classic plugin
    #[command(
        subcommand,
        after_help = "Examples:\n  lrcli plugin install\n  lrcli plugin uninstall\n  lrcli plugin path"
    )]
    Plugin(PluginCommand),

    /// Run the long-lived broker (started by the plugin; not for direct use)
    #[command(hide = true)]
    Broker,

    /// Send a raw command to the plugin and print the reply (debugging)
    #[command(hide = true)]
    Raw {
        cmd: String,
        /// JSON object with the command arguments
        args: Option<String>,
    },
}

#[derive(Subcommand)]
enum PluginCommand {
    /// Symlink the plugin into Lightroom Classic's Modules directory
    #[command(after_help = "\
Links plugin/lrcli.lrdevplugin into
~/Library/Application Support/Adobe/Lightroom/Modules and writes
~/.lrcli/config.json, which tells the plugin where this lrcli binary lives so
it can start the broker. Run it again after moving the lrcli binary.

An existing correct link is left alone. Anything else at the target is
refused unless --force is given.

After installing, restart Lightroom Classic, then open File > Plug-in Manager
and select lrcli once: Lightroom only starts plugins when they are first used.

Examples:
  lrcli plugin install
  lrcli plugin install --force
  lrcli --json plugin install

JSON output:
  {\"ok\":true,\"action\":\"installed\",\"source\":\"...\",\"target\":\"...\",
   \"config\":\"...\",\"brokerPath\":\"...\"}
  action is one of installed, replaced, unchanged.")]
    Install {
        /// Replace whatever is already at the target
        #[arg(long)]
        force: bool,
    },

    /// Remove the plugin symlink and config.json
    #[command(after_help = "\
Removes only the symlink that lrcli created (never a real directory) and
~/.lrcli/config.json. Restart Lightroom Classic to unload the plugin.

Examples:
  lrcli plugin uninstall
  lrcli --json plugin uninstall

JSON output:
  {\"ok\":true,\"action\":\"removed\",\"target\":\"...\",\"config\":\"...\"}
  action is removed or absent.")]
    Uninstall,

    /// Print where the plugin source, install target and config live
    #[command(after_help = "\
Examples:
  lrcli plugin path
  lrcli --json plugin path

JSON output:
  {\"ok\":true,\"source\":\"...\",\"target\":\"...\",\"config\":\"...\"}")]
    Path,
}

fn parse_secs(text: &str) -> Result<f64, String> {
    text.parse::<f64>()
        .ok()
        .filter(|secs| secs.is_finite() && *secs > 0.0)
        .ok_or_else(|| "must be a positive number of seconds".to_string())
}

fn json_requested() -> bool {
    std::env::args().skip(1).any(|a| a == "--json")
        || std::env::var("LRCLI_OUTPUT").is_ok_and(|v| v == "json")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) => {
            let _ = e.print();
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            if json_requested() {
                let message = e.render().to_string();
                output::emit_error(&Error::usage(message.trim()), true);
            } else {
                let _ = e.print();
            }
            return ExitCode::from(2);
        }
    };
    let json = cli.json || json_requested();
    ExitCode::from(run(cli, json).await)
}

async fn run(cli: Cli, json: bool) -> u8 {
    let paths = match Paths::from_env() {
        Ok(paths) => paths,
        Err(e) => {
            output::emit_error(&e, json);
            return e.exit_code();
        }
    };
    let ctx = Context {
        paths: paths.clone(),
        timeout: Duration::from_secs_f64(cli.timeout),
        connect_timeout: Duration::from_secs_f64(cli.connect_timeout),
    };
    match cli.command {
        Command::Status => output::emit(commands::status::run(&ctx).await, json, output::status),
        Command::Raw { cmd, args } => output::emit(
            commands::raw::run(&ctx, &cmd, args.as_deref()).await,
            json,
            output::raw,
        ),
        Command::Plugin(PluginCommand::Install { force }) => output::emit(
            commands::plugin::install(&paths, force),
            json,
            output::plugin_install,
        ),
        Command::Plugin(PluginCommand::Uninstall) => output::emit(
            commands::plugin::uninstall(&paths),
            json,
            output::plugin_uninstall,
        ),
        Command::Plugin(PluginCommand::Path) => {
            output::emit(commands::plugin::path(&paths), json, output::plugin_path)
        }
        Command::Broker => match broker::run(&paths).await {
            Ok(()) => 0,
            Err(e) => e.exit_code(),
        },
    }
}
