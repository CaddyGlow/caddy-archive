fn main() {
    loop {
        honggfuzz::fuzz!(|data: &[u8]| {
            archive_fuzz::archives::archive(data);
        });
    }
}
