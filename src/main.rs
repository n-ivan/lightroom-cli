mod output;

use std::process::ExitCode;
use std::time::Duration;

use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand};

use lrcli::broker;
use lrcli::commands::photos::{self, Rating, SearchOptions};
use lrcli::commands::preview::{self, PreviewOptions};
use lrcli::commands::{self, Context};
use lrcli::error::Error;
use lrcli::paths::Paths;

const DEFAULT_TIMEOUT_SECS: f64 = 30.0;

const ABOUT_EXAMPLES: &str = "\
Examples:
  lrcli status                 Check that Lightroom Classic is reachable
  lrcli --json status          Same, as JSON for scripts and agents
  lrcli plugin install         Link the plugin into Lightroom Classic
  lrcli search --rating '>=4' --flag picked
                               Find photos in the catalog
  lrcli info --photo 123       Show a photo's metadata
  lrcli preview --photo 123    Render a JPEG preview of a photo with its edits

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

    /// Seconds to wait for each request to be answered [default: 30; `preview` scales with the batch]
    #[arg(long, global = true, value_name = "SECS", value_parser = parse_secs)]
    timeout: Option<f64>,

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

    /// Find photos in the catalog by rating, flag, label, keyword, date and more
    #[command(after_help = "\
Filters are combined with AND. With no filters every photo in the catalog
matches. Results are paged: --limit (default 50, max 500) and --offset.

--rating takes a number from 0 to 5, optionally after an operator:
  3 (same as ==3), >=3, <=2, >3, <3, !=0
Quote operators in the shell: --rating '>=3'.

--since and --until are inclusive capture dates (YYYY-MM-DD). --keyword may be
repeated; a photo must have every keyword. --text searches all searchable
fields. --filename, --folder and --camera match substrings; --collection
matches a collection name.

--ids prints only photo ids, one per line, so the output can feed other
commands.

Examples:
  lrcli search --rating '>=4' --flag picked
  lrcli search --keyword portrait --keyword studio --since 2024-01-01
  lrcli search --camera R5 --limit 100 --offset 100
  lrcli search --label red --ids | lrcli select --photos -
  lrcli search --folder 2024-05 --json | lrcli info --photos -

JSON output:
  {\"ok\":true,\"total\":123,\"offset\":0,\"limit\":50,\"photos\":[
    {\"id\":42,\"filename\":\"IMG_0001.CR3\",\"path\":\"/photos/IMG_0001.CR3\",
     \"captureTime\":\"2024-05-31T10:00:00Z\",\"rating\":4,\"flag\":\"picked\",
     \"label\":\"red\",\"width\":6000,\"height\":4000}]}
  With --ids: {\"ok\":true,\"ids\":[42],\"total\":123}")]
    Search(Box<SearchArgs>),

    /// Select photos in Lightroom Classic's Library
    #[command(after_help = "\
Replaces Lightroom's current selection with the given photos. Give ids with
repeated --photo, or pass --photos - to read them from stdin (one id per line,
or the JSON output of `lrcli search`). If a Library filter or the current
source hides the photos, the source is switched to All Photographs.

Examples:
  lrcli select --photo 42 --photo 43
  lrcli search --rating 5 --ids | lrcli select --photos -

JSON output:
  {\"ok\":true,\"selected\":[42,43],\"sourceChanged\":false}

Fails with photo_not_found if an id is not in the catalog, and select_failed if
Lightroom still would not select every photo.")]
    Select(TargetArgs),

    /// List the photos currently selected in Lightroom Classic
    #[command(after_help = "\
Prints the same table as `lrcli search`, or only ids with --ids. An empty
selection is not an error.

Examples:
  lrcli selection
  lrcli selection --ids
  lrcli --json selection

JSON output:
  {\"ok\":true,\"photos\":[{\"id\":42,\"filename\":\"IMG_0001.CR3\",...}]}
  With --ids: {\"ok\":true,\"ids\":[42],\"total\":1}")]
    Selection {
        /// Print only photo ids, one per line
        #[arg(long)]
        ids: bool,
    },

    /// Show metadata, EXIF and develop settings for photos
    #[command(after_help = "\
Without --photo or --photos, acts on Lightroom's current selection. Ids that
are not in the catalog fail the whole command (photo_not_found). If nothing is
selected and no ids are given, it fails with no_photo_selected.

Human output omits develop settings unless --develop is given; --json always
includes them.

Examples:
  lrcli info
  lrcli info --photo 42 --develop
  lrcli search --flag picked --json | lrcli info --photos -
  lrcli --json info --photo 42

JSON output:
  {\"ok\":true,\"target\":\"ids\",\"photos\":[{\"id\":42,\"filename\":\"...\",
   \"rating\":4,\"flag\":\"picked\",\"label\":null,\"title\":null,\"caption\":null,
   \"keywords\":[\"a\"],\"fileFormat\":\"RAW\",
   \"exif\":{\"camera\":\"...\",\"lens\":null,\"iso\":100,\"shutter\":\"1/250 sec\",
           \"aperture\":\"f/2.8\",\"focalLength\":\"50 mm\",\"exposureBias\":null},
   \"gps\":null,\"develop\":{...}}]}
  target is \"selection\" when no ids were given.")]
    Info {
        #[command(flatten)]
        target: TargetArgs,

        /// Include develop settings in human output (always included in JSON)
        #[arg(long)]
        develop: bool,
    },

    /// Render JPEG previews of photos (with current edits) and report their stats
    #[command(after_help = "\
Renders each photo to a JPEG you can open or view directly, so you can judge
an edit without leaving the terminal. Without --photo or --photos, acts on
Lightroom's current selection (at most 50 photos per call). Files are written
to $LRCLI_HOME/previews/<YYYYmmdd-HHMMSS>-<pid>/ (UTC; ~/.lrcli by default).
Only the 20 most recent batch directories are kept.

By default Lightroom exports each photo (sRGB, JPEG quality 80, long edge
--size pixels, no sharpening, location stripped). This always reflects the
current edits but takes about 1-2 seconds per photo. With --fast, Lightroom's
cached preview is used instead: it is quick, but it can be stale after recent
edits and may not match --size, so every result is marked possiblyStale.

Unless --no-stats is given, each preview is analysed: width and height, mean
luminance (0 to 1, Rec. 709 weights on sRGB values), and clipping as the
percentage of pixels at 255 (highlights) or 0 (shadows) per channel and in
luminance. --histogram adds 256-bin histograms (luma, r, g, b). Human output
shows the worst clipped channel as `hi` and `lo`.

The request timeout defaults to 30 s plus 5 s per photo (120 s for the
selection); --timeout overrides it. A photo that fails (for example a video)
is reported on its own line and does not stop the others. Exit 0 if at least
one preview was produced; exit 1 if every photo failed.

Examples:
  lrcli preview --photo 149
  lrcli preview --photo 149 --photo 150 --size 2048
  lrcli search --rating '>=4' --ids | lrcli preview --photos -
  lrcli preview --fast --no-stats
  lrcli --json preview --photo 149 --histogram

JSON output:
  {\"ok\":true,\"target\":\"ids\",\"mode\":\"export\",\"size\":1600,
   \"dir\":\"/Users/me/.lrcli/previews/20240531-100000-123\",
   \"photos\":[{\"filename\":\"IMG_0001.CR3\",\"id\":149,\"error\":null,
     \"file\":\"/Users/me/.lrcli/previews/20240531-100000-123/lrcli-IMG_0001.jpg\",
     \"stats\":{\"width\":1600,\"height\":1067,\"meanLuminance\":0.42,
       \"clipping\":{\"highlights\":{\"r\":0.3,\"g\":0.1,\"b\":0.0,\"luma\":0.0},
                    \"shadows\":{\"r\":1.2,\"g\":0.9,\"b\":1.1,\"luma\":0.8}}}}]}
  mode is \"thumbnail\" with --fast, and each photo then has \"possiblyStale\":true.
  A photo that failed has \"file\":null and an \"error\" message. \"histograms\"
  is added under \"stats\" with --histogram.")]
    Preview(Box<PreviewArgs>),

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

#[derive(Args)]
struct SearchArgs {
    /// Rating filter: N, or an operator and N (==, !=, >, <, >=, <=), 0-5
    #[arg(long, value_name = "EXPR", value_parser = photos::parse_rating, allow_hyphen_values = true)]
    rating: Option<Rating>,

    /// Pick flag
    #[arg(long, value_parser = ["picked", "rejected", "unflagged"])]
    flag: Option<String>,

    /// Color label
    #[arg(long, value_parser = ["red", "yellow", "green", "blue", "purple"])]
    label: Option<String>,

    /// Keyword the photo must have (repeatable)
    #[arg(long = "keyword", value_name = "KEYWORD")]
    keywords: Vec<String>,

    /// Match text in any searchable field
    #[arg(long, value_name = "TEXT")]
    text: Option<String>,

    /// File name contains
    #[arg(long, value_name = "TEXT")]
    filename: Option<String>,

    /// Folder path contains
    #[arg(long, value_name = "TEXT")]
    folder: Option<String>,

    /// Collection name
    #[arg(long, value_name = "NAME")]
    collection: Option<String>,

    /// Captured on or after this date
    #[arg(long, value_name = "YYYY-MM-DD", value_parser = photos::parse_date)]
    since: Option<String>,

    /// Captured on or before this date
    #[arg(long, value_name = "YYYY-MM-DD", value_parser = photos::parse_date)]
    until: Option<String>,

    /// Camera model contains
    #[arg(long, value_name = "TEXT")]
    camera: Option<String>,

    /// Maximum photos to return
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=500))]
    limit: Option<u32>,

    /// Number of matching photos to skip
    #[arg(long, value_name = "N")]
    offset: Option<u32>,

    /// Print only photo ids, one per line
    #[arg(long)]
    ids: bool,
}

#[derive(Args)]
struct PreviewArgs {
    #[command(flatten)]
    target: TargetArgs,

    /// Long edge of the preview in pixels (64 to 4096)
    #[arg(long, value_name = "N", default_value_t = preview::DEFAULT_SIZE, value_parser = clap::value_parser!(u32).range(64..=4096))]
    size: u32,

    /// Use Lightroom's cached preview: quick, but can be stale after recent edits
    #[arg(long)]
    fast: bool,

    /// Include 256-bin histograms (luma, r, g, b) in the stats
    #[arg(long)]
    histogram: bool,

    /// Skip decoding the previews and computing stats
    #[arg(long)]
    no_stats: bool,
}

#[derive(Args)]
struct TargetArgs {
    /// Photo id (repeatable)
    #[arg(long = "photo", value_name = "ID")]
    photo: Vec<i64>,

    /// Read photo ids from stdin: one per line, or JSON from `lrcli search --json`
    #[arg(long, value_name = "-", value_parser = ["-"], conflicts_with = "photo")]
    photos: Option<String>,
}

impl TargetArgs {
    fn ids(&self) -> Result<Option<Vec<i64>>, Error> {
        if self.photos.is_some() {
            return photos::read_stdin_ids().map(Some);
        }
        Ok((!self.photo.is_empty()).then(|| self.photo.clone()))
    }
}

impl From<SearchArgs> for SearchOptions {
    fn from(args: SearchArgs) -> Self {
        Self {
            rating: args.rating,
            flag: args.flag,
            label: args.label,
            keywords: args.keywords,
            text: args.text,
            filename: args.filename,
            folder: args.folder,
            collection: args.collection,
            since: args.since,
            until: args.until,
            camera: args.camera,
            limit: args.limit,
            offset: args.offset,
        }
    }
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
    let mut ctx = Context {
        paths: paths.clone(),
        timeout: Duration::from_secs_f64(cli.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS)),
        connect_timeout: Duration::from_secs_f64(cli.connect_timeout),
    };
    match cli.command {
        Command::Status => output::emit(commands::status::run(&ctx).await, json, output::status),
        Command::Search(args) => {
            let ids = args.ids;
            let result = commands::photos::search(&ctx, &(*args).into(), ids).await;
            output::emit(result, json, if ids { output::ids } else { output::search })
        }
        Command::Select(target) => {
            let result = match target.ids() {
                Ok(Some(ids)) => commands::photos::select(&ctx, &ids).await,
                Ok(None) => Err(Error::usage(
                    "Give the photos to select with --photo <id> (repeatable) or --photos - to read ids from stdin.",
                )),
                Err(e) => Err(e),
            };
            output::emit(result, json, output::select)
        }
        Command::Selection { ids } => {
            let result = commands::photos::selection(&ctx, ids).await;
            output::emit(
                result,
                json,
                if ids { output::ids } else { output::selection },
            )
        }
        Command::Info { target, develop } => {
            let result = match target.ids() {
                Ok(ids) => commands::photos::info(&ctx, ids.as_deref(), develop || json).await,
                Err(e) => Err(e),
            };
            output::emit(result, json, output::info)
        }
        Command::Preview(args) => {
            let result = match args.target.ids() {
                Ok(ids) => {
                    ctx.timeout = cli.timeout.map_or_else(
                        || preview::default_timeout(ids.as_deref()),
                        Duration::from_secs_f64,
                    );
                    let options = PreviewOptions {
                        ids,
                        size: args.size,
                        fast: args.fast,
                        histogram: args.histogram,
                        stats: !args.no_stats,
                    };
                    preview::run(&ctx, &options).await
                }
                Err(e) => Err(e),
            };
            output::emit(result, json, output::preview)
        }
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
