fn main() {
    loop {
        honggfuzz::fuzz!(|data: &[u8]| {
            archive_fuzz::cab(data);
        });
    }
}
