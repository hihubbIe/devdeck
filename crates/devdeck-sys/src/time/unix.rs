pub fn local_hms(ms: u64) -> Option<(u8, u8, u8)> {
    let secs = (ms / 1000) as libc::time_t;
    // SAFETY: an all-zero `tm` is valid, and localtime_r only writes into the one we own.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return None;
    }
    Some((tm.tm_hour as u8, tm.tm_min as u8, tm.tm_sec as u8))
}
