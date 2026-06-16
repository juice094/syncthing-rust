use std::time::SystemTime;

pub(crate) fn fmt_iso8601(t: SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Utc> = t.into();
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}
