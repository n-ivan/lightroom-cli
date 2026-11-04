use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Context, status};
use crate::error::{Error, Kind};
use crate::platform;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Validates a user-supplied catalog path and returns it canonicalised.
pub fn check_catalog(path: &Path) -> Result<PathBuf, Error> {
    if path.extension().and_then(|e| e.to_str()) != Some("lrcat") {
        return Err(Error::usage(format!(
            "{} is not a Lightroom catalog. Give the path of a .lrcat file.",
            path.display()
        )));
    }
    path.canonicalize().map_err(|_| {
        Error::usage(format!(
            "Catalog {} does not exist. Check the path.",
            path.display()
        ))
    })
}

/// Compares paths after resolving symlinks and `..`; falls back to the raw
/// text when a path can't be resolved (e.g. the catalog's volume is gone).
pub fn same_catalog(a: &Path, b: &Path) -> bool {
    let resolve = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    resolve(a) == resolve(b)
}

/// Polls `probe` every `interval` until it returns `Some` or `total` elapses.
async fn poll_until<T, F, Fut>(total: Duration, interval: Duration, mut probe: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let deadline = Instant::now() + total;
    loop {
        if let Some(found) = probe().await {
            return Some(found);
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        tokio::time::sleep(interval.min(deadline - now)).await;
    }
}

/// Short timeouts so a half-started plugin can't stall the wait loop.
fn probe_context(ctx: &Context) -> Context {
    Context {
        paths: ctx.paths.clone(),
        timeout: PROBE_TIMEOUT,
        connect_timeout: ctx.connect_timeout.min(Duration::from_secs(1)),
    }
}

async fn hello_if_answering(ctx: &Context) -> Result<Option<Value>, Error> {
    match status::run(ctx).await {
        Ok(reply) => Ok(Some(reply)),
        Err(e) if matches!(e.kind, Kind::Unreachable | Kind::Timeout) => Ok(None),
        Err(e) => Err(e),
    }
}

fn with_started(mut reply: Value, started: bool) -> Value {
    reply["started"] = json!(started);
    reply
}

pub async fn start(ctx: &Context, catalog: Option<&Path>, wait: Duration) -> Result<Value, Error> {
    let catalog = catalog.map(check_catalog).transpose()?;
    if let Some(reply) = hello_if_answering(ctx).await? {
        if let (Some(wanted), Some(open)) = (
            &catalog,
            reply.pointer("/catalog/path").and_then(Value::as_str),
        ) && !same_catalog(wanted, Path::new(open))
        {
            return Err(Error::failed(
                "catalog_mismatch",
                format!(
                    "Lightroom Classic has {open} open, not {}. Run `lrcli restart --catalog {}` to switch.",
                    wanted.display(),
                    wanted.display()
                ),
            ));
        }
        return Ok(with_started(reply, false));
    }
    if platform::app_is_running()? {
        platform::activate_app()?;
    } else {
        platform::launch_app(catalog.as_deref())?;
    }
    let probe = probe_context(ctx);
    let reply = poll_until(wait, POLL_INTERVAL, || async {
        hello_if_answering(&probe).await.ok().flatten()
    })
    .await;
    match reply {
        Some(reply) => Ok(with_started(reply, true)),
        None => Err(Error::new(
            Kind::Unreachable,
            "start_timeout",
            format!(
                "Lightroom Classic is running but the lrcli plugin didn't answer within {} s. Check Lightroom for a dialog (catalog upgrade, update prompt, \"What's New\") or open Library > Plug-in Extras > lrcli Status…. If the plugin isn't installed, run `lrcli plugin install`, then `lrcli restart`.",
                wait.as_secs()
            ),
        )),
    }
}

pub async fn stop(ctx: &Context, wait: Duration) -> Result<Value, Error> {
    if !platform::app_is_running()? {
        return Ok(json!({ "ok": true, "stopped": false }));
    }
    platform::quit_app()?;
    let probe = probe_context(ctx);
    let gone = poll_until(wait, POLL_INTERVAL, || async {
        let exited = !platform::app_is_running().unwrap_or(true)
            && matches!(hello_if_answering(&probe).await, Ok(None));
        exited.then_some(())
    })
    .await;
    if gone.is_some() {
        Ok(json!({ "ok": true, "stopped": true }))
    } else {
        Err(Error::failed(
            "stop_timeout",
            format!(
                "Lightroom Classic didn't quit within {} s. It may be showing a dialog (for example a catalog backup); answer it in Lightroom, or raise --wait.",
                wait.as_secs()
            ),
        ))
    }
}

pub async fn restart(
    ctx: &Context,
    catalog: Option<&Path>,
    stop_wait: Duration,
    start_wait: Duration,
) -> Result<Value, Error> {
    let catalog = catalog.map(check_catalog).transpose()?;
    stop(ctx, stop_wait).await?;
    start(ctx, catalog.as_deref(), start_wait).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn catalog_must_be_an_existing_lrcat_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let good = dir.path().join("a.lrcat");
        std::fs::write(&good, "").unwrap();
        let other = dir.path().join("a.txt");
        std::fs::write(&other, "").unwrap();
        assert!(check_catalog(&good).is_ok());
        assert_eq!(check_catalog(&other).unwrap_err().exit_code(), 2);
        assert_eq!(
            check_catalog(&dir.path().join("missing.lrcat"))
                .unwrap_err()
                .exit_code(),
            2
        );
    }

    #[test]
    fn catalogs_compare_after_canonicalising() {
        let dir = tempfile::TempDir::new().unwrap();
        let a = dir.path().join("a.lrcat");
        std::fs::write(&a, "").unwrap();
        let indirect = dir.path().join("sub/../a.lrcat");
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        assert!(same_catalog(&a, &indirect));
        assert!(!same_catalog(&a, &dir.path().join("b.lrcat")));
        assert!(same_catalog(
            Path::new("/gone/x.lrcat"),
            Path::new("/gone/x.lrcat")
        ));
    }

    #[tokio::test]
    async fn poll_stops_as_soon_as_the_probe_succeeds() {
        let calls = Cell::new(0);
        let ok = poll_until(Duration::from_secs(5), Duration::from_millis(1), || {
            calls.set(calls.get() + 1);
            let done = calls.get() == 3;
            async move { done.then_some("ok") }
        })
        .await;
        assert_eq!(ok, Some("ok"));
        assert_eq!(calls.get(), 3);
    }

    #[tokio::test]
    async fn poll_gives_up_after_the_deadline() {
        let started = Instant::now();
        let ok = poll_until(
            Duration::from_millis(50),
            Duration::from_millis(10),
            || async { None::<()> },
        )
        .await;
        assert!(ok.is_none());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
