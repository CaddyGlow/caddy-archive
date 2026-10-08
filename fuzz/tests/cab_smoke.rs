#[test]
fn retained_cab_fixtures_exercise_the_bounded_reader() {
    archive_fuzz::cab(include_bytes!("fixtures/cab-quantum/mszip_lzx_qtm.cab"));
    archive_fuzz::cab(include_bytes!(
        "fixtures/cab-quantum/cve-2014-9556-qtm-infinite-loop.cab"
    ));
}
