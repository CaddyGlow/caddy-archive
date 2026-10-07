fn main() {
    loop {
        honggfuzz::fuzz!(|data: &[u8]| {
            let _ = archive_fuzz::udf::read(data);
        });
    }
}
