//! The test clock the server reads under `failpoints` (`TILR_TEST_TIME_FILE`).

/// Publish `us` (Unix microseconds) as the servers' test clock: a rename, so
/// a reader (the server or a pane process) sees one whole value. The next
/// protocol response is the test's ordering barrier.
pub fn set_test_time(path: &std::path::Path, us: u64) {
    let next = path.with_extension("next");
    std::fs::write(&next, us.to_string()).unwrap();
    std::fs::rename(next, path).unwrap();
}
