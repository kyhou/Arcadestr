//! Completion means a successful refresh, not merely that relay work stopped.
//! Call after emitting and persisting any partial listings.

pub(crate) fn finish(result: Result<u32, String>, on_success: impl FnOnce()) -> Result<(), String> {
    result?;
    on_success();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::finish;
    use std::cell::Cell;

    #[test]
    fn successful_empty_refresh_emits_completion() {
        let completed = Cell::new(false);
        assert!(finish(Ok(0), || completed.set(true)).is_ok());
        assert!(completed.get());
    }

    #[test]
    fn failed_timed_out_and_partial_refreshes_preserve_error_without_completion() {
        for message in [
            "Relay fetch incomplete: 0 succeeded, 1 failed, 0 timed out; 0 events retained (partial results)",
            "Relay fetch incomplete: 0 succeeded, 0 failed, 1 timed out; 0 events retained (partial results)",
            "Relay fetch incomplete: 1 succeeded, 1 failed, 0 timed out; 3 events retained (partial results)",
        ] {
            let completed = Cell::new(false);
            assert_eq!(finish(Err(message.into()), || completed.set(true)), Err(message.into()));
            assert!(!completed.get());
        }
    }
}
