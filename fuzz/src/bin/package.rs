fn main() {
    loop {
        honggfuzz::fuzz!(|data: &[u8]| {
            archive_fuzz::archives::package(data);
        });
    }
}
