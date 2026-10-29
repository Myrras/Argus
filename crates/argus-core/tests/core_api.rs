//! The `Core` entry points' edges: unknown ids, bad input, and the poll
//! loop across a settings change.

mod support;

use std::sync::mpsc;
use std::time::Duration;

use argus_core::provider::ProviderId;
use argus_core::{Core, CoreError, ReviewId, ThreadId};

use support::*;

#[test]
fn unknown_ids_are_not_found() {
    let core = Core::default();
    let id = ReviewId::from_key("github|github.com|acme-fixture/nothing|1");
    assert!(matches!(core.review(id), Err(CoreError::NotFound)));
    assert!(matches!(core.threads(id), Err(CoreError::NotFound)));
    assert!(matches!(core.mark_reviewed(id), Err(CoreError::NotFound)));
    assert!(matches!(
        core.list_files(id, false),
        Err(CoreError::NotFound)
    ));
    assert!(matches!(
        core.file_at(id, "c1", PATH),
        Err(CoreError::NotFound)
    ));
    assert!(matches!(
        core.set_file_viewed(id, PATH, true),
        Err(CoreError::NotFound)
    ));
    assert!(matches!(
        core.commentable_lines(id, PATH),
        Err(CoreError::NotFound)
    ));
    assert!(matches!(core.ai_state(id), Err(CoreError::NotFound)));
    assert!(matches!(
        core.review_histories(id),
        Err(CoreError::NotFound)
    ));
    let thread = ThreadId::new();
    assert!(matches!(
        core.thread_review(thread),
        Err(CoreError::NotFound)
    ));
    assert_eq!(core.cached_snapshot(thread).unwrap(), None);
    assert!(core.reply_to_thread(thread, "Hi").is_err());
}

#[test]
fn files_outside_the_diff_are_refused() {
    let p = Platform::new();
    let core = core_over(&p);
    let id = the_review(&core).id;
    assert!(matches!(
        core.set_file_viewed(id, "README.md", true),
        Err(CoreError::InvalidInput(_))
    ));
    // Clearing a mark that was never set is fine.
    assert!(core
        .set_file_viewed(id, "README.md", false)
        .unwrap()
        .is_empty());
    assert!(matches!(
        core.commentable_lines(id, "README.md"),
        Err(CoreError::InvalidInput(_))
    ));
    // Never reviewed: there is no "since last review".
    assert!(matches!(
        core.list_files(id, true),
        Err(CoreError::InvalidInput(_))
    ));
}

#[test]
fn a_settings_change_restarts_the_poll_loop() {
    let p = Platform::new();
    let core = core_over(&p);
    // Not polling: a nudge does nothing.
    core.poll_now();

    let (tx, rx) = mpsc::channel();
    core.start_polling(move |r| {
        let _ = tx.send(r.refreshed_at);
    });
    let first = rx.recv_timeout(Duration::from_secs(10)).unwrap();

    // github.com off: the loop restarts over the new (empty) provider
    // list, with the same callback.
    core.update_settings(host_off(&core, ProviderId::Github, "github.com"))
        .unwrap();
    let after = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(after >= first);
    let calls = p.calls().len();
    core.poll_now();
    rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(p.calls().len(), calls, "the old provider is not polled");
    core.stop_polling();
    core.poll_now();
    assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
}
