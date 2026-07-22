use compass_whisper::utils::{compression_ratio, format_timestamp};

#[test]
fn compression_and_timestamp_helpers_cover_empty_repetitive_hour_and_custom_marker_shapes() {
    assert_eq!(compression_ratio(""), 0.0);
    assert!(compression_ratio(&"compass ".repeat(100)) > 5.0);
    assert_eq!(format_timestamp(0.0, false, "."), "00:00.000");
    assert_eq!(format_timestamp(1.2345, true, ","), "00:00:01,235");
    assert_eq!(format_timestamp(3_661.001, false, "."), "01:01:01.001");
}

#[test]
#[should_panic(expected = "non-negative timestamp expected")]
fn negative_timestamps_are_rejected() {
    let _ = format_timestamp(-0.001, false, ".");
}
