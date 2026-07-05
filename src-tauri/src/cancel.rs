//! Cooperative cancellation for long-running ffmpeg subprocess work.
//!
//! A single `Arc<AtomicBool>` (owned by `AppState`) is shared by every
//! cancellable operation. Top-level commands reset it to `false` at entry;
//! library code only ever reads it and, on cancel, kills its ffmpeg child so
//! the CPU isn't pegged to completion on a mistaken long render/burn.
//!
//! The 100ms poll cadence in `output_with_cancel` matches the streaming
//! stderr-tail loop in `video_engine::extract_frames_fixed_interval`; both are
//! well below any human-perceptible "why won't Cancel respond" threshold while
//! adding no measurable overhead to the common (non-cancelled) path.

use std::process::{Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use crate::error::NarratorError;

/// True when a cancel flag is present and set.
pub(crate) fn is_cancelled(cancel_flag: &Option<Arc<AtomicBool>>) -> bool {
    cancel_flag
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::SeqCst))
}

/// Return `Cancelled` if the flag is set — a cheap checkpoint helper so loops
/// don't sprout repeated `match` blocks.
pub(crate) fn check_cancelled(cancel_flag: &Option<Arc<AtomicBool>>) -> Result<(), NarratorError> {
    if is_cancelled(cancel_flag) {
        return Err(NarratorError::Cancelled);
    }
    Ok(())
}

/// Kill a child and reap it. Used when a cancel is observed mid-run so the
/// ffmpeg process doesn't linger.
pub(crate) async fn kill_child_after_cancel(child: &mut Child) {
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// Run a one-shot ffmpeg command (`.output()`-style) while polling the cancel
/// flag every 100ms. On cancel the child is killed and `Cancelled` returned.
///
/// `kill_on_drop(true)` is set so that even if this future is dropped (e.g. the
/// caller is itself cancelled) the child is reaped rather than orphaned.
pub(crate) async fn output_with_cancel(
    cmd: &mut Command,
    cancel: &Option<Arc<AtomicBool>>,
) -> Result<Output, NarratorError> {
    check_cancelled(cancel)?;
    // Match `Command::output()` semantics: capture both streams. Draining them
    // on separate tasks means `child.wait()` can never deadlock against a full
    // stdout/stderr pipe (ffmpeg's showinfo/silencedetect output is chatty).
    cmd.kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| NarratorError::FfmpegFailed(format!("spawn: {e}")))?;

    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(s) = stdout.as_mut() {
            let _ = s.read_to_end(&mut buf).await;
        }
        buf
    });
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(s) = stderr.as_mut() {
            let _ = s.read_to_end(&mut buf).await;
        }
        buf
    });

    loop {
        tokio::select! {
            status = child.wait() => {
                let status = status.map_err(|e| NarratorError::FfmpegFailed(format!("wait: {e}")))?;
                let stdout = stdout_task.await.unwrap_or_default();
                let stderr = stderr_task.await.unwrap_or_default();
                return Ok(Output { status, stdout, stderr });
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if is_cancelled(cancel) {
                    kill_child_after_cancel(&mut child).await;
                    stdout_task.abort();
                    stderr_task.abort();
                    return Err(NarratorError::Cancelled);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_cancelled_passes_when_absent_or_false() {
        assert!(check_cancelled(&None).is_ok());
        assert!(check_cancelled(&Some(Arc::new(AtomicBool::new(false)))).is_ok());
    }

    #[test]
    fn check_cancelled_errors_when_set() {
        let flag = Some(Arc::new(AtomicBool::new(true)));
        assert!(matches!(
            check_cancelled(&flag),
            Err(NarratorError::Cancelled)
        ));
    }

    #[tokio::test]
    async fn output_with_cancel_returns_cancelled_when_preset() {
        let flag = Some(Arc::new(AtomicBool::new(true)));
        // `true` is a trivial always-succeeds binary; the pre-check should fire
        // before it is ever spawned.
        let mut cmd = Command::new("true");
        let err = output_with_cancel(&mut cmd, &flag).await.unwrap_err();
        assert!(matches!(err, NarratorError::Cancelled));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn output_with_cancel_kills_quiet_running_child() {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_for_task = flag.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            flag_for_task.store(true, Ordering::SeqCst);
        });
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let err = output_with_cancel(&mut cmd, &Some(flag)).await.unwrap_err();
        assert!(matches!(err, NarratorError::Cancelled));
    }

    #[tokio::test]
    async fn output_with_cancel_returns_output_on_success() {
        let mut cmd = Command::new("true");
        let out = output_with_cancel(&mut cmd, &None).await.unwrap();
        assert!(out.status.success());
    }
}
