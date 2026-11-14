//! Completion means a successful refresh, not merely that relay work stopped.
//! Call after emitting and persisting any partial listings.

pub(crate) fn finish<E>(result: Result<u32, E>, on_success: impl FnOnce()) -> Result<(), E> {
    result?;
    on_success();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::finish;
    use arcadestr_core::marketplace_fetch::MarketplaceFetchError;
    use std::cell::Cell;

    #[test]
    fn successful_empty_refresh_emits_completion() {
        let completed = Cell::new(false);
        assert!(finish(Ok::<u32, MarketplaceFetchError>(0), || completed.set(true)).is_ok());
        assert!(completed.get());
    }

    #[test]
    fn unreachable_and_failed_refreshes_preserve_error_without_completion() {
        for error in [
            MarketplaceFetchError::Unreachable {
                failed: 1,
                timed_out: 0,
            },
            MarketplaceFetchError::Unreachable {
                failed: 0,
                timed_out: 1,
            },
            MarketplaceFetchError::failed("Connection error: no relays"),
        ] {
            let completed = Cell::new(false);
            assert_eq!(
                finish(Err::<u32, _>(error.clone()), || completed.set(true)),
                Err(error)
            );
            assert!(!completed.get());
        }
    }

    #[test]
    fn unreachable_refresh_crosses_the_bridge_as_a_tagged_object() {
        let wire = serde_json::to_value(MarketplaceFetchError::Unreachable {
            failed: 2,
            timed_out: 1,
        })
        .expect("serializable");
        assert_eq!(wire["kind"], "unreachable");
        assert_eq!(wire["failed"], 2);
    }
}
