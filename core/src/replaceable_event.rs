/// Return whether a replaceable event supersedes the current event.
///
/// Newer timestamps win. Equal timestamps are ordered by lexicographically
/// lower event IDs, with a present candidate ID preferred over a missing one.
pub fn is_replaceable_event_newer(
    candidate_created_at: u64,
    candidate_event_id: Option<&str>,
    current_created_at: u64,
    current_event_id: Option<&str>,
) -> bool {
    match candidate_created_at.cmp(&current_created_at) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match (candidate_event_id, current_event_id) {
            (Some(candidate), Some(current)) => candidate < current,
            (Some(_), None) => true,
            (None, _) => false,
        },
    }
}
