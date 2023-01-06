use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn get_system_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        // This is safe, it only fails if the current system time is before the
        // UNIX_EPOCH. So it will only fail if a time traveler from 1970 is
        // using it.
        .unwrap()
        .as_secs()
}
