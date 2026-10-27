//! A daemon-free circuit count for the pre-install verdict.
//!
//! The Unit-1 cap counts *mapped* circuits (deduped by circuit hash); on a
//! v2 box the closest in-file proxy is the `ShapedDevices.csv` data-row
//! count, which this module reports. The mapped-vs-raw decision belongs to
//! the caller (the card's §1.3).

use std::path::Path;

/// Count the data rows of a `ShapedDevices.csv` byte slice.
///
/// The header is consumed and `#`-comment lines are skipped, matching the
/// real loader's reader settings (`lqos_config::ConfigShapedDevices`), so the
/// number agrees with what the shape compiler will see.
pub fn count_shaped_devices_bytes(bytes: &[u8]) -> Result<usize, String> {
    let mut reader = csv::ReaderBuilder::new()
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .flexible(true)
        .from_reader(bytes);
    reader
        .headers()
        .map_err(|e| format!("cannot read the CSV header: {e}"))?;
    let mut count = 0usize;
    for record in reader.records() {
        record.map_err(|e| format!("cannot read a CSV row: {e}"))?;
        count += 1;
    }
    Ok(count)
}

/// Count the data rows of a `ShapedDevices.csv` file.
pub fn count_shaped_devices(path: &Path) -> Result<usize, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    count_shaped_devices_bytes(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = b"\
Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Sparking Min,Sparking Max,IPv4,IPv6,Download Min,Upload Min,Download Max,Upload Max
1,Circuit 1,1,Device 1,,, ,192.168.101.2,,25,5,10000,10000
2,Circuit 2,2,Device 2,,, ,192.168.101.3,,25,5,10000,10000
# a comment row that must not count
3,Circuit 3,3,Device 3,,, ,192.168.101.4,,25,5,10000,10000
";

    #[test]
    fn counts_only_data_rows() {
        assert_eq!(count_shaped_devices_bytes(FIXTURE).unwrap(), 3);
    }

    #[test]
    fn header_only_is_zero() {
        let header = b"Circuit ID,Circuit Name\n";
        assert_eq!(count_shaped_devices_bytes(header).unwrap(), 0);
    }

    #[test]
    fn a_missing_file_is_an_error() {
        let err = count_shaped_devices(Path::new("/nonexistent/ShapedDevices.csv")).unwrap_err();
        assert!(err.starts_with("cannot read"), "unexpected error: {err}");
    }
}
