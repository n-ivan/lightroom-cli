mod output;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand, ValueEnum};

use serde_json::json;

use lrcli::broker;
use lrcli::commands::edit::{self, Run};
use lrcli::commands::export::{self, ExportOptions, Size};
use lrcli::commands::photos::{self, Rating, SearchOptions};
use lrcli::commands::preview::{self, PreviewOptions};
use lrcli::commands::{self, Context};
use lrcli::compare;
use lrcli::error::Error;
use lrcli::history::command_line;
use lrcli::paths::Paths;
use lrcli::settings::{Setting, parse_setting};

const DEFAULT_TIMEOUT_SECS: f64 = 30.0;
const STOP_WAIT_SECS: f64 = 60.0;

const ABOUT_EXAMPLES: &str = "\
Examples:
  lrcli status                 Check that Lightroom Classic is reachable
  lrcli --json status          Same, as JSON for scripts and agents
  lrcli plugin install         Link the plugin into Lightroom Classic
  lrcli start                  Launch Lightroom Classic and wait for the plugin
  lrcli restart                Quit and relaunch Lightroom Classic
  lrcli search --rating '>=4' --flag picked
                               Find photos in the catalog
  lrcli info --photo 123       Show a photo's metadata
  lrcli preview --photo 123    Render a JPEG preview of a photo with its edits
  lrcli apply --photo 123 Exposure2012+=0.5
                               Edit develop settings (`lrcli undo` reverts it)

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

    /// Seconds to wait for each request to be answered [default: 30; `preview` and `export` scale with the batch]
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

    /// Launch Lightroom Classic and wait until the plugin answers
    #[command(after_help = "\
Does nothing (exit 0) if the plugin already answers. Otherwise Lightroom
Classic is launched in the background (with --catalog if given) and lrcli
waits up to --wait seconds (default 120) for the plugin to answer, then prints
the same output as `status`. Lightroom only starts plugins once its window has
been active, so lrcli activates it for a fraction of a second and returns
focus to the app you were using (repeated every 15 s while waiting);
--foreground leaves Lightroom in front instead.

--catalog must be an existing .lrcat file (exit 2 otherwise). If Lightroom
already has a different catalog open, start fails (exit 1): use
`lrcli restart --catalog PATH` to switch. macOS only.

The first time lrcli controls Lightroom, macOS asks whether your terminal app
may automate Lightroom Classic (System Settings > Privacy & Security >
Automation). Allow it.

Examples:
  lrcli start
  lrcli start --catalog ~/Pictures/Lightroom/main.lrcat --wait 300
  lrcli --json start

JSON output:
  The `status` reply plus \"started\": true if this call launched or woke
  Lightroom, false if the plugin was already answering.

Exit 3 means the plugin didn't answer in time: check Lightroom for a dialog
(catalog upgrade, update prompt, \"What's New\") or open File > Plug-in
Extras > lrcli Status..., and run `lrcli plugin install` if the plugin isn't
installed.")]
    Start {
        #[command(flatten)]
        launch: LaunchArgs,
    },

    /// Quit Lightroom Classic and wait until it has exited
    #[command(after_help = "\
Asks Lightroom Classic to quit (like Cmd+Q) and waits up to --wait seconds
(default 60) for the app to exit and the broker to disappear. It never
force-kills: if Lightroom shows a dialog (for example a catalog backup), answer
it, or the command fails with exit 1 after --wait. Exit 0 and \"not running\"
if Lightroom wasn't running. macOS only; the first quit triggers the macOS
Automation permission prompt for your terminal app.

Examples:
  lrcli stop
  lrcli stop --wait 300

JSON output:
  {\"ok\":true,\"stopped\":true}; \"stopped\" is false if it wasn't running.")]
    Stop {
        /// Seconds to wait for Lightroom Classic to exit
        #[arg(long, value_name = "SECS", default_value = "60", value_parser = parse_secs)]
        wait: f64,
    },

    /// Quit and relaunch Lightroom Classic (picks up new plugin files)
    #[command(after_help = "\
Runs `lrcli stop`, then `lrcli start`. Use it after `lrcli plugin install` or
after adding plugin files, because Lightroom only loads new plugin files at
startup. Without --catalog, Lightroom opens its default catalog. --wait is the
time to wait for the plugin after relaunching (default 120); quitting waits up
to 60 s. macOS only; see `lrcli stop` for the Automation permission prompt.

Examples:
  lrcli restart
  lrcli restart --catalog ~/Pictures/Lightroom/other.lrcat

JSON output:
  Same as `lrcli start`.")]
    Restart {
        #[command(flatten)]
        launch: LaunchArgs,
    },

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

    /// List develop setting names by panel with their valid ranges
    #[command(after_help = "\
Prints the develop settings `lrcli apply` accepts, grouped by Lightroom panel
(Basic, Tone Curve, Color Mixer, Color Grading, Detail, Effects, Lens
Corrections, Calibration, Crop), each with its valid range. Crop edges are
fractions (0 to 1) of the original frame.

Without --photo the ranges are for raw files, and a `non-raw` range is shown
where non-raw files differ. Temperature/Tint only apply to raw files; non-raw
files (JPEG, TIFF, PNG...) use IncrementalTemperature/IncrementalTint instead.
With --photo the ranges are those of that photo's file type, the keys that do
not apply to it are left out, and each setting shows its current value in
brackets. --panel limits the output to one panel, given by id or title
(case-insensitive, a unique prefix is enough).

Examples:
  lrcli settings
  lrcli settings --panel basic
  lrcli settings --panel color --photo 149
  lrcli --json settings --panel crop

JSON output:
  {\"ok\":true,\"fileType\":\"raw\",\"photo\":149,\"panels\":[{\"id\":\"adjustPanel\",
    \"title\":\"Basic\",\"keys\":[{\"key\":\"Exposure2012\",\"min\":-5,\"max\":5,\"value\":0.5}]}]}
  fileType is null without --photo; then \"value\" is absent, a key's
  \"nonRaw\":{\"min\",\"max\"} appears where non-raw files differ, and
  \"nonRawOnly\":true marks keys that exist only for non-raw files.")]
    Settings {
        /// Only this panel (id or title, case-insensitive)
        #[arg(long, value_name = "NAME")]
        panel: Option<String>,

        /// Show ranges for this photo's file type and its current values
        #[arg(long, value_name = "ID")]
        photo: Option<i64>,
    },

    /// Compose a before/after image from two JPEG files
    #[command(after_help = "\
Scales both images to the same height (the smaller of the two, at most 1600
px) and writes them side by side as one JPEG, with BEFORE on the left and
AFTER on the right. Prints the output path and, for each input, its size, mean
luminance and clipping (see `lrcli preview --help`). Does not need Lightroom.
Without --out the file is written next to AFTER as <after-stem>-compare.jpg.

Examples:
  lrcli compare before.jpg after.jpg
  lrcli --json compare before.jpg after.jpg --out /tmp/side-by-side.jpg

JSON output:
  {\"ok\":true,\"file\":\"/tmp/side-by-side.jpg\",
   \"before\":{\"file\":\"before.jpg\",\"stats\":{...}},
   \"after\":{\"file\":\"after.jpg\",\"stats\":{...}}}")]
    Compare {
        /// The image shown on the left
        #[arg(value_name = "BEFORE")]
        before: PathBuf,

        /// The image shown on the right
        #[arg(value_name = "AFTER")]
        after: PathBuf,

        /// Where to write the composite (default: next to AFTER)
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
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

    /// Export photos to image files (JPEG, TIFF, PSD, DNG or the original) in a folder
    #[command(after_help = "\
Exports each photo through Lightroom's own export, with its current edits, into
the folder given by --dest (created if missing; relative paths are resolved
against the current directory). Without --photo or --photos, acts on
Lightroom's current selection (at most 500 photos per call). Videos are not
exported: they get an error of their own.

Defaults: JPEG, quality 90, full size, sRGB, all metadata, file name
{{image_name}}, existing files renamed (never overwritten). Quality applies to
JPEG only; --bit-depth to TIFF and PSD; --tiff-compression to TIFF. The size
options are mutually exclusive and never enlarge a photo; --size WxH fits the
photo inside that box. Sharpening is for the output media given by --sharpen
(--sharpen-amount defaults to standard).

--on-conflict decides what happens when a file already exists: rename (the
default, Lightroom adds -2, -3, ...), overwrite, or skip (the photo is then
reported without a file). Lightroom's dialog is never shown.

--name is Lightroom's file name template. Common tokens:
  {{image_name}}                       file name without extension (default)
  {{image_originalName}}               original file name
  {{image_folder}}                     name of the photo's folder
  {{copy_name}}                        virtual copy name
  {{date_YYYYMMDD}}                    capture date, also {{date_YYYY}}, {{date_MM}}, {{date_DD}}
  {{naming_sequenceNumber_3Digits}}    001, 002, ... (also 1Digit to 5Digits)
  {{naming_operationSequence_3Digits}} image number within this export
  {{com.adobe.title}}                  title (other metadata tokens work too)
The extension is added by Lightroom, in lowercase. {{custom_token}} is not
supported. --subfolder puts the files in a subfolder of --dest.

--add-to-catalog imports the exported files into the catalog. That changes the
catalog and cannot be undone with `lrcli undo`.

The request timeout defaults to 60 s plus 10 s per photo (300 s for the
selection); --timeout overrides it. A photo that fails is reported on its own
line and does not stop the others. Exit 0 if at least one photo was exported;
exit 1 if every photo failed.

Examples:
  lrcli export --photo 149 --dest ~/Exports
  lrcli export --photo 149 --photo 150 --dest ~/Exports/web --long-edge 2048 --quality 85
  lrcli export --dest ~/Exports/print --format tiff --bit-depth 16 --color-space adobergb --tiff-compression zip
  lrcli export --photo 149 --dest ~/Exports --size 1920x1080 --sharpen screen --remove-location
  lrcli export --photo 149 --dest ~/Exports --name '{{date_YYYYMMDD}}-{{naming_sequenceNumber_3Digits}}'
  lrcli search --flag picked --ids | lrcli export --photos - --dest ~/Exports/picks

JSON output:
  {\"ok\":true,\"target\":\"ids\",\"dest\":\"/Users/me/Exports\",
   \"photos\":[{\"id\":149,\"filename\":\"IMG_0001.CR3\",
     \"file\":\"/Users/me/Exports/IMG_0001.jpg\",\"bytes\":3355443,\"error\":null}]}
  A photo that failed has \"file\":null and an \"error\" message.")]
    Export(Box<ExportArgs>),

    /// Change develop settings (sliders) on photos
    #[command(after_help = "\
Sets or adjusts develop settings on photos. Without --photo or --photos, acts
on Lightroom's current selection. Each SETTING is one of:
  Key=Value    set an absolute value (number, true/false, or text)
  Key+=N       add N to the current value
  Key-=N       subtract N from the current value
Keys are Lightroom's develop setting names, for example Exposure2012,
Contrast2012, Highlights2012, Shadows2012, Whites2012, Blacks2012, Texture,
Clarity2012, Dehaze, Vibrance, Saturation, Temperature, Tint, WhiteBalance.
Run `lrcli settings` to list the names by panel with their valid ranges (add
--photo ID for current values), or `lrcli info --develop` for every key a photo
holds. Unknown names are refused with close matches. Relative changes are computed by lrcli from the
current value; Lightroom may clamp the result, and the output flags any value
that differs from what was requested.

All photos are written in one step that shows as \"lrcli apply\" in Lightroom's
Edit > Undo menu. Videos are reported per photo and skipped. If every photo
fails the command exits 1. --preview then renders a preview of the edited
photos (like `lrcli preview`). --compare renders a preview before and after the
write and saves one image per photo with the before on the left and the after on
the right, plus mean luminance and clipping before and after (it takes the place
of --preview). Every successful edit is recorded for `lrcli undo`.

Examples:
  lrcli apply --photo 149 Exposure2012+=0.5 Contrast2012=15 WhiteBalance=Auto
  lrcli apply Highlights2012-=20 Shadows2012+=30 --preview
  lrcli apply --photo 149 Exposure2012+=0.5 --compare
  lrcli search --rating 5 --ids | lrcli apply --photos - Vibrance=25

JSON output:
  {\"ok\":true,\"target\":\"ids\",\"photos\":[{\"id\":149,\"filename\":\"IMG_0001.CR3\",
    \"before\":{\"Exposure2012\":0.0},\"after\":{\"Exposure2012\":0.5},
    \"requested\":{\"Exposure2012\":0.5}}]}
  A photo that failed has \"error\" instead. With --preview the preview reply is
  under \"preview\" (or \"previewError\"). With --compare each photo has
  \"compare\":{\"file\":COMPOSITE,\"before\":{\"file\":...,\"stats\":{...}},
  \"after\":{...}} (or \"compareError\"); a failed render is a top-level
  \"compareError\":{\"error\",\"message\"}. The write is never failed by --compare.")]
    Apply(Box<ApplyArgs>),

    /// List the develop presets available in Lightroom
    #[command(after_help = "\
Prints each preset folder and the presets in it. Use a preset's name or uuid
with `lrcli preset`.

Examples:
  lrcli presets
  lrcli --json presets

JSON output:
  {\"ok\":true,\"folders\":[{\"name\":\"Color\",\"presets\":[{\"name\":\"Vivid\",\"uuid\":\"...\"}]}]}")]
    Presets,

    /// Apply a develop preset to photos
    #[command(after_help = "\
NAME_OR_UUID is a preset name (matched exactly, ignoring case) or its uuid
(see `lrcli presets --json`). A name shared by several presets is refused with
the choices listed as Folder/Name (uuid). --amount is the preset strength, 0 to
200 (100 is the preset as designed). Without --photo or --photos, acts on
Lightroom's current selection. The output lists, per photo, the settings the
preset changed. Shows as \"lrcli preset <name>\" in Lightroom's Edit > Undo menu,
and is recorded for `lrcli undo`. --compare saves a before/after image per
photo (left = before, right = after) with luminance and clipping stats, as
described in `lrcli apply --help`.

Examples:
  lrcli preset Vivid --photo 149
  lrcli preset \"Warm Contrast\" --amount 60 --preview
  lrcli preset Vivid --photo 149 --compare
  lrcli preset 1C2D3E4F-0123-4567-89AB-CDEF01234567 --photos -

JSON output:
  {\"ok\":true,\"target\":\"ids\",\"preset\":{\"name\":\"Vivid\",\"uuid\":\"...\",\"folder\":\"Color\"},
   \"photos\":[{\"id\":149,\"filename\":\"IMG_0001.CR3\",
     \"before\":{\"Vibrance\":0},\"after\":{\"Vibrance\":30}}]}")]
    Preset(Box<PresetArgs>),

    /// Run Lightroom's Develop actions (auto tone, auto white balance, resets)
    #[command(after_help = "\
ACTION is one or more of auto-tone, auto-wb (auto white balance), as-shot-wb
(back to the camera's white balance) and reset-all (every develop adjustment);
they run in the order given. --reset KEY resets single adjustments to their
defaults (repeat it or separate keys with commas, e.g. --reset Exposure2012,Tint)
and runs after the ACTIONs. At least one action or --reset is required. Without
--photo or --photos, acts on Lightroom's current selection.

Lightroom has to be in the Develop module for these to work, so while this runs
it switches to Develop, selects each photo in turn, and afterwards restores the
previous module and selection. It works with Lightroom in the background but
may take a few seconds per photo, and the screen visibly changes meanwhile. The
output lists, per photo, the settings that changed. The edits are recorded for
`lrcli undo`. --preview and --compare work as in `lrcli apply --help`.

Examples:
  lrcli develop auto-tone --photo 149
  lrcli develop auto-wb auto-tone --compare
  lrcli develop as-shot-wb --photos -
  lrcli develop --reset Exposure2012,Contrast2012 --photo 149
  lrcli develop reset-all --photo 149

JSON output:
  {\"ok\":true,\"target\":\"ids\",\"photos\":[{\"id\":149,\"filename\":\"IMG_0001.CR3\",
    \"before\":{\"Exposure2012\":0.0},\"after\":{\"Exposure2012\":0.4}}]}
  A photo that failed has \"error\" instead; one that changed nothing has empty
  before and after.")]
    Develop(Box<DevelopArgs>),

    /// Switch Lightroom to a module (library, develop, map, ...)
    #[command(after_help = "\
NAME is one of library, develop, map, book, slideshow, print, web.

Examples:
  lrcli module develop
  lrcli module library

JSON output:
  {\"ok\":true,\"previous\":\"library\",\"current\":\"develop\"}")]
    Module {
        /// Module name
        #[arg(value_name = "NAME", value_parser = ["library", "develop", "map", "book", "slideshow", "print", "web"])]
        name: String,
    },

    /// Add or remove keywords on photos
    #[command(subcommand)]
    Tag(TagCommand),

    /// Set the star rating of photos
    #[command(after_help = "\
RATING is 0 (no stars) to 5. Without --photo or --photos, acts on Lightroom's
current selection. Recorded for `lrcli undo`.

Examples:
  lrcli rate 4 --photo 149
  lrcli rate 0

JSON output:
  {\"ok\":true,\"target\":\"ids\",\"photos\":[{\"id\":149,\"filename\":\"IMG_0001.CR3\",\"before\":2,\"after\":4}]}")]
    Rate(Box<RateArgs>),

    /// Pick, reject or unflag photos
    #[command(after_help = "\
Without --photo or --photos, acts on Lightroom's current selection. Recorded
for `lrcli undo`.

Examples:
  lrcli flag picked --photo 149
  lrcli flag none
  lrcli search --rating 1 --ids | lrcli flag rejected --photos -

JSON output:
  {\"ok\":true,\"target\":\"ids\",\"photos\":[{\"id\":149,\"filename\":\"IMG_0001.CR3\",
    \"before\":\"none\",\"after\":\"picked\"}]}")]
    Flag(Box<FlagArgs>),

    /// Set the color label of photos
    #[command(after_help = "\
Lightroom's color labels are red, yellow, green, blue and purple; `none`
removes the label. Without --photo or --photos, acts on Lightroom's current
selection. Recorded for `lrcli undo`.

Examples:
  lrcli label red --photo 149
  lrcli label none

JSON output:
  {\"ok\":true,\"target\":\"ids\",\"photos\":[{\"id\":149,\"filename\":\"IMG_0001.CR3\",
    \"before\":\"none\",\"after\":\"red\"}]}")]
    Label(Box<LabelArgs>),

    /// Undo the last recorded lrcli edits
    #[command(after_help = "\
Every successful apply, preset, develop, tag, rate, flag and label is recorded in
$LRCLI_HOME/history.jsonl together with the values it replaced and the values it
produced. `undo` reverts the most recent --steps entries (default 1) that were
made in the catalog Lightroom has open, newest first, then removes them from the
history. Entries for other catalogs are skipped.

`undo` restores the values recorded before the lrcli command, for exactly the
settings that command changed (only changes are recorded; `lrcli history` shows
what undo will take back next). It does not use Lightroom's own undo.

A photo that was changed since lrcli edited it (any recorded setting no longer
holds the value lrcli set) is not touched: undo stops, exits 1 with
`undo_conflict`, restores no photo of that step and keeps the step in the
history. --force restores anyway. Develop settings that had no value before
cannot be cleared and are reported as warnings. If a photo or keyword no longer
exists, everything else is still restored and the problem is reported per photo.
Undo steps are not themselves recorded.

--dry-run prints what would be restored without changing anything and reports
conflicts of the newest step (later steps depend on the earlier ones being undone).

Examples:
  lrcli undo
  lrcli undo --steps 3 --dry-run
  lrcli undo --force

JSON output:
  {\"ok\":true,\"dryRun\":false,\"undone\":[{\"time\":\"...\",\"command\":\"lrcli rate 4\",
    \"changes\":[...],\"photos\":[{\"id\":149,\"filename\":\"IMG_0001.CR3\",\"restored\":[\"rating\"],
    \"warnings\":[],\"conflict\":[]}]}]}
  With --dry-run: {\"ok\":true,\"dryRun\":true,\"undo\":[{\"time\",\"command\",\"changes\",
    \"photos\":[{\"id\",\"filename\",\"conflict\":[\"Exposure2012 is 0.6, lrcli set 0.5\"]}]}]}
  (\"photos\" only on the first entry). On a conflict the error is `undo_conflict`.")]
    Undo {
        /// Number of recorded edits to undo, newest first
        #[arg(long, value_name = "N", default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
        steps: u32,

        /// Show what would be restored without changing anything
        #[arg(long)]
        dry_run: bool,

        /// Restore photos even if they changed since lrcli edited them
        #[arg(long)]
        force: bool,
    },

    /// List recorded edits (what `lrcli undo` would revert)
    #[command(after_help = "\
Shows the most recent entries of $LRCLI_HOME/history.jsonl, newest first, for
every catalog. This does not need Lightroom to be running. Only the newest 500
entries are kept; older ones are dropped when a new edit is recorded.

Examples:
  lrcli history
  lrcli --json history --limit 5

JSON output:
  {\"ok\":true,\"entries\":[{\"time\":\"2024-05-31T10:00:00Z\",\"command\":\"lrcli rate 4\",
    \"catalog\":\"/path/main.lrcat\",\"changes\":[{\"id\":149,\"rating\":2}]}]}")]
    History {
        /// Maximum entries to show
        #[arg(long, value_name = "N", default_value_t = 20)]
        limit: usize,
    },

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

After installing, run `lrcli restart` (or `lrcli start` if Lightroom Classic is
not running) so Lightroom loads the plugin.

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

#[derive(Subcommand)]
enum TagCommand {
    /// Add keywords to photos
    #[command(after_help = "\
A KEYWORD is a name or a path like \"Parent > Child\" (quote it). A bare name
matches a keyword anywhere in the keyword tree; if several keywords share the
name the command fails and lists their paths. Missing keywords, including
missing parents in a path, are created. Without --photo or --photos, acts on
Lightroom's current selection. Recorded for `lrcli undo`.

Examples:
  lrcli tag add portrait --photo 149
  lrcli tag add \"Places > France > Paris\" studio --photos -

JSON output:
  {\"ok\":true,\"target\":\"ids\",\"photos\":[{\"id\":149,\"filename\":\"IMG_0001.CR3\",
    \"added\":[\"portrait\"],\"removed\":[]}]}
  Only keywords that actually changed are listed.")]
    Add {
        /// Keyword name or "Parent > Child" path
        #[arg(required = true, value_name = "KEYWORD")]
        keywords: Vec<String>,

        #[command(flatten)]
        target: TargetArgs,
    },

    /// Remove keywords from photos
    #[command(after_help = "\
Keywords are given as for `tag add`. A keyword that does not exist in the
catalog is an error; a photo that doesn't have it is left alone. The keyword
itself stays in the catalog. Recorded for `lrcli undo`.

Examples:
  lrcli tag remove portrait --photo 149
  lrcli tag remove \"Places > France\"

JSON output:
  Same as `tag add`.")]
    Remove {
        /// Keyword name or "Parent > Child" path
        #[arg(required = true, value_name = "KEYWORD")]
        keywords: Vec<String>,

        #[command(flatten)]
        target: TargetArgs,
    },
}

#[derive(Args)]
struct ApplyArgs {
    #[command(flatten)]
    target: TargetArgs,

    /// Key=Value, Key+=N or Key-=N (see --help)
    #[arg(required = true, value_name = "SETTING", value_parser = parse_setting)]
    settings: Vec<Setting>,

    /// After writing, render a preview of the edited photos
    #[arg(long)]
    preview: bool,

    /// Render before and after previews and save a side-by-side image (left = before, right = after)
    #[arg(long)]
    compare: bool,
}

#[derive(Args)]
struct PresetArgs {
    /// Preset name or uuid
    #[arg(value_name = "NAME_OR_UUID")]
    preset: String,

    #[command(flatten)]
    target: TargetArgs,

    /// Preset strength, 0 to 200
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(0..=200))]
    amount: Option<u32>,

    /// After writing, render a preview of the edited photos
    #[arg(long)]
    preview: bool,

    /// Render before and after previews and save a side-by-side image (left = before, right = after)
    #[arg(long)]
    compare: bool,
}

#[derive(Args)]
struct DevelopArgs {
    /// auto-tone, auto-wb, as-shot-wb or reset-all (run in order)
    #[arg(value_name = "ACTION", value_parser = ["auto-tone", "auto-wb", "as-shot-wb", "reset-all"])]
    actions: Vec<String>,

    /// Reset a develop setting to its default (repeatable, or comma-separated)
    #[arg(long = "reset", value_name = "KEY", value_delimiter = ',')]
    reset: Vec<String>,

    #[command(flatten)]
    target: TargetArgs,

    /// After writing, render a preview of the edited photos
    #[arg(long)]
    preview: bool,

    /// Render before and after previews and save a side-by-side image (left = before, right = after)
    #[arg(long)]
    compare: bool,
}

#[derive(Args)]
struct RateArgs {
    /// Stars, 0 to 5
    #[arg(value_name = "RATING", value_parser = clap::value_parser!(u8).range(0..=5))]
    rating: u8,

    #[command(flatten)]
    target: TargetArgs,
}

#[derive(Args)]
struct FlagArgs {
    /// Flag to set
    #[arg(value_parser = ["picked", "rejected", "none"])]
    flag: String,

    #[command(flatten)]
    target: TargetArgs,
}

#[derive(Args)]
struct LabelArgs {
    /// Color label to set
    #[arg(value_parser = ["red", "yellow", "green", "blue", "purple", "none"])]
    label: String,

    #[command(flatten)]
    target: TargetArgs,
}

#[derive(Args)]
struct LaunchArgs {
    /// Catalog (.lrcat) to open when launching Lightroom Classic
    #[arg(long, value_name = "PATH")]
    catalog: Option<PathBuf>,

    /// Seconds to wait for the plugin to answer
    #[arg(long, value_name = "SECS", default_value = "120", value_parser = parse_secs)]
    wait: f64,

    /// Leave Lightroom Classic in front instead of returning focus to the
    /// current app after the brief activation the plugin needs to start
    #[arg(long)]
    foreground: bool,
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

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum FormatArg {
    Jpeg,
    Tiff,
    Psd,
    Dng,
    Original,
}

impl FormatArg {
    fn wire(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Tiff => "TIFF",
            Self::Psd => "PSD",
            Self::Dng => "DNG",
            Self::Original => "ORIGINAL",
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum ColorSpaceArg {
    Srgb,
    Adobergb,
    Prophoto,
}

impl ColorSpaceArg {
    fn wire(self) -> &'static str {
        match self {
            Self::Srgb => "sRGB",
            Self::Adobergb => "AdobeRGB",
            Self::Prophoto => "ProPhotoRGB",
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum TiffCompressionArg {
    None,
    Lzw,
    Zip,
}

impl TiffCompressionArg {
    fn wire(self) -> &'static str {
        match self {
            Self::None => "compressionMethod_None",
            Self::Lzw => "compressionMethod_LZW",
            Self::Zip => "compressionMethod_ZIP",
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum SharpenArg {
    Screen,
    Matte,
    Glossy,
}

impl SharpenArg {
    fn wire(self) -> &'static str {
        match self {
            Self::Screen => "screen",
            Self::Matte => "matte",
            Self::Glossy => "glossy",
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum SharpenAmountArg {
    Low,
    Standard,
    High,
}

impl SharpenAmountArg {
    fn level(self) -> u8 {
        match self {
            Self::Low => 1,
            Self::Standard => 2,
            Self::High => 3,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum MetadataArg {
    All,
    Copyright,
    CopyrightContact,
    AllExceptCamera,
}

impl MetadataArg {
    fn wire(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Copyright => "copyrightOnly",
            Self::CopyrightContact => "copyrightAndContactOnly",
            Self::AllExceptCamera => "allExceptCameraInfo",
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum ConflictArg {
    Rename,
    Overwrite,
    Skip,
}

impl ConflictArg {
    fn wire(self) -> &'static str {
        match self {
            Self::Rename => "rename",
            Self::Overwrite => "overwrite",
            Self::Skip => "skip",
        }
    }
}

#[derive(Args)]
#[group(multiple = false)]
struct SizeArgs {
    /// Scale so the long edge is at most this many pixels
    #[arg(long, value_name = "PX", value_parser = clap::value_parser!(u32).range(1..=i64::from(export::MAX_PIXELS)))]
    long_edge: Option<u32>,

    /// Scale so the short edge is at most this many pixels
    #[arg(long, value_name = "PX", value_parser = clap::value_parser!(u32).range(1..=i64::from(export::MAX_PIXELS)))]
    short_edge: Option<u32>,

    /// Fit inside a box of this width and height in pixels, e.g. 1920x1080
    #[arg(long, value_name = "WxH", value_parser = parse_dimensions)]
    size: Option<(u32, u32)>,

    /// Scale to this many megapixels
    #[arg(long, value_name = "N", value_parser = parse_megapixels)]
    megapixels: Option<f64>,
}

#[derive(Args)]
struct ExportArgs {
    #[command(flatten)]
    target: TargetArgs,

    /// Folder to export into (created if missing)
    #[arg(long, value_name = "DIR")]
    dest: PathBuf,

    /// File format [default: jpeg]
    #[arg(long, value_enum)]
    format: Option<FormatArg>,

    /// JPEG quality, 0 to 100 (jpeg only) [default: 90]
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=100))]
    quality: Option<u8>,

    #[command(flatten)]
    size: SizeArgs,

    /// Color space
    #[arg(long, value_enum, default_value = "srgb")]
    color_space: ColorSpaceArg,

    /// Bit depth (tiff and psd only)
    #[arg(long, value_name = "BITS", value_parser = ["8", "16"])]
    bit_depth: Option<String>,

    /// TIFF compression (tiff only)
    #[arg(long, value_enum)]
    tiff_compression: Option<TiffCompressionArg>,

    /// Sharpen for this output media
    #[arg(long, value_enum)]
    sharpen: Option<SharpenArg>,

    /// Sharpening amount
    #[arg(long, value_enum, default_value = "standard", requires = "sharpen")]
    sharpen_amount: SharpenAmountArg,

    /// Metadata to embed
    #[arg(long, value_enum, default_value = "all")]
    metadata: MetadataArg,

    /// Strip GPS location from the exported files
    #[arg(long)]
    remove_location: bool,

    /// File name template, see the tokens below
    #[arg(long, value_name = "TEMPLATE", default_value = export::DEFAULT_NAME)]
    name: String,

    /// What to do when a file already exists
    #[arg(long, value_enum, default_value = "rename")]
    on_conflict: ConflictArg,

    /// Put the files in this subfolder of --dest
    #[arg(long, value_name = "NAME")]
    subfolder: Option<String>,

    /// Import the exported files into the catalog (changes the catalog; not undoable)
    #[arg(long)]
    add_to_catalog: bool,
}

impl ExportArgs {
    fn options(&self, ids: Option<Vec<i64>>) -> Result<ExportOptions, Error> {
        let format = self.format.unwrap_or(FormatArg::Jpeg);
        if self.quality.is_some() && format != FormatArg::Jpeg {
            return Err(Error::usage("--quality applies only to --format jpeg."));
        }
        let deep = matches!(format, FormatArg::Tiff | FormatArg::Psd);
        if self.bit_depth.is_some() && !deep {
            return Err(Error::usage(
                "--bit-depth applies only to --format tiff or psd.",
            ));
        }
        if self.tiff_compression.is_some() && format != FormatArg::Tiff {
            return Err(Error::usage(
                "--tiff-compression applies only to --format tiff.",
            ));
        }
        let size = &self.size;
        Ok(ExportOptions {
            ids,
            dest: self.dest.clone(),
            format: format.wire(),
            quality: (format == FormatArg::Jpeg)
                .then(|| self.quality.unwrap_or(export::DEFAULT_QUALITY)),
            size: size
                .long_edge
                .map(Size::LongEdge)
                .or(size.short_edge.map(Size::ShortEdge))
                .or(size.size.map(|(w, h)| Size::Dimensions(w, h)))
                .or(size.megapixels.map(Size::Megapixels)),
            color_space: self.color_space.wire(),
            bit_depth: self.bit_depth.as_deref().and_then(|bits| bits.parse().ok()),
            tiff_compression: self.tiff_compression.map(TiffCompressionArg::wire),
            sharpen: self
                .sharpen
                .map(|media| (media.wire(), self.sharpen_amount.level())),
            metadata: self.metadata.wire(),
            remove_location: self.remove_location,
            name: self.name.clone(),
            on_conflict: self.on_conflict.wire(),
            subfolder: self.subfolder.clone(),
            add_to_catalog: self.add_to_catalog,
        })
    }
}

fn parse_dimensions(text: &str) -> Result<(u32, u32), String> {
    let invalid = || "must look like 1920x1080".to_string();
    let (width, height) = text.split_once(['x', 'X']).ok_or_else(invalid)?;
    let pixels = |part: &str| {
        part.parse::<u32>()
            .ok()
            .filter(|px| (1..=export::MAX_PIXELS).contains(px))
            .ok_or_else(|| format!("each side must be 1 to {}", export::MAX_PIXELS))
    };
    Ok((pixels(width)?, pixels(height)?))
}

fn parse_megapixels(text: &str) -> Result<f64, String> {
    text.parse::<f64>()
        .ok()
        .filter(|mp| mp.is_finite() && *mp > 0.0 && *mp <= export::MAX_MEGAPIXELS)
        .ok_or_else(|| format!("must be above 0 and at most {}", export::MAX_MEGAPIXELS))
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

fn write_run(command_line: &str, preview: bool, compare: bool, timeout: Option<f64>) -> Run<'_> {
    Run {
        command_line,
        preview,
        compare,
        timeout: timeout.map(Duration::from_secs_f64),
    }
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
    let command_line = command_line(std::env::args().skip(1));
    match cli.command {
        Command::Status => output::emit(commands::status::run(&ctx).await, json, output::status),
        Command::Start { launch } => output::emit(
            commands::lifecycle::start(
                &ctx,
                launch.catalog.as_deref(),
                Duration::from_secs_f64(launch.wait),
                launch.foreground,
            )
            .await,
            json,
            output::start,
        ),
        Command::Stop { wait } => output::emit(
            commands::lifecycle::stop(&ctx, Duration::from_secs_f64(wait)).await,
            json,
            output::stop,
        ),
        Command::Restart { launch } => output::emit(
            commands::lifecycle::restart(
                &ctx,
                launch.catalog.as_deref(),
                Duration::from_secs_f64(STOP_WAIT_SECS),
                Duration::from_secs_f64(launch.wait),
                launch.foreground,
            )
            .await,
            json,
            output::start,
        ),
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
        Command::Settings { panel, photo } => output::emit(
            commands::photos::settings(&ctx, photo, panel.as_deref()).await,
            json,
            output::settings,
        ),
        Command::Compare { before, after, out } => {
            let out = out.unwrap_or_else(|| compare::default_output(&after));
            let result = compare::compare_files(&before, &after, &out).map(|mut reply| {
                reply["ok"] = json!(true);
                reply
            });
            output::emit(result, json, output::compare)
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
        Command::Export(args) => {
            let result = match args.target.ids() {
                Ok(ids) => {
                    ctx.timeout = cli.timeout.map_or_else(
                        || export::default_timeout(ids.as_deref()),
                        Duration::from_secs_f64,
                    );
                    match args.options(ids) {
                        Ok(options) => export::run(&ctx, &options).await,
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(e),
            };
            output::emit(result, json, output::export)
        }
        Command::Apply(args) => {
            let run = write_run(&command_line, args.preview, args.compare, cli.timeout);
            let result = match args.target.ids() {
                Ok(ids) => edit::apply(&ctx, &run, ids.as_deref(), &args.settings).await,
                Err(e) => Err(e),
            };
            output::emit(result, json, output::apply)
        }
        Command::Presets => output::emit(edit::presets(&ctx).await, json, output::presets),
        Command::Preset(args) => {
            let run = write_run(&command_line, args.preview, args.compare, cli.timeout);
            let result = match args.target.ids() {
                Ok(ids) => {
                    edit::preset(&ctx, &run, ids.as_deref(), &args.preset, args.amount).await
                }
                Err(e) => Err(e),
            };
            output::emit(result, json, output::preset)
        }
        Command::Develop(args) => {
            let run = write_run(&command_line, args.preview, args.compare, cli.timeout);
            let result = match (
                args.target.ids(),
                edit::develop_actions(&args.actions, &args.reset),
            ) {
                (Ok(ids), Ok(actions)) => {
                    ctx.timeout = cli.timeout.map_or_else(
                        || preview::default_timeout(ids.as_deref()),
                        Duration::from_secs_f64,
                    );
                    edit::develop(&ctx, &run, ids.as_deref(), actions).await
                }
                (Err(e), _) | (_, Err(e)) => Err(e),
            };
            output::emit(result, json, output::apply)
        }
        Command::Module { name } => output::emit(
            commands::photos::module(&ctx, &name).await,
            json,
            output::module,
        ),
        Command::Tag(tag) => {
            let run = write_run(&command_line, false, false, cli.timeout);
            let (target, add, remove) = match tag {
                TagCommand::Add { keywords, target } => (target, keywords, Vec::new()),
                TagCommand::Remove { keywords, target } => (target, Vec::new(), keywords),
            };
            let result = match target.ids() {
                Ok(ids) => edit::tag(&ctx, &run, ids.as_deref(), &add, &remove).await,
                Err(e) => Err(e),
            };
            output::emit(result, json, output::tag)
        }
        Command::Rate(args) => {
            let run = write_run(&command_line, false, false, cli.timeout);
            let result = match args.target.ids() {
                Ok(ids) => edit::rate(&ctx, &run, ids.as_deref(), args.rating).await,
                Err(e) => Err(e),
            };
            output::emit(result, json, output::scalar)
        }
        Command::Flag(args) => {
            let run = write_run(&command_line, false, false, cli.timeout);
            let result = match args.target.ids() {
                Ok(ids) => edit::flag(&ctx, &run, ids.as_deref(), &args.flag).await,
                Err(e) => Err(e),
            };
            output::emit(result, json, output::scalar)
        }
        Command::Label(args) => {
            let run = write_run(&command_line, false, false, cli.timeout);
            let result = match args.target.ids() {
                Ok(ids) => edit::label(&ctx, &run, ids.as_deref(), &args.label).await,
                Err(e) => Err(e),
            };
            output::emit(result, json, output::scalar)
        }
        Command::Undo {
            steps,
            dry_run,
            force,
        } => output::emit(
            edit::undo(&ctx, steps as usize, dry_run, force).await,
            json,
            output::undo,
        ),
        Command::History { limit } => {
            output::emit(edit::history(&ctx, limit), json, output::history)
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
