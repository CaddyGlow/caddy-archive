//! WinZip AES primitives. Password derivation has a fixed 1000-round format cost.
use crate::{Error, Result};
use aes::cipher::{KeyIvInit, StreamCipher};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use std::io::{Read, Seek, SeekFrom};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, Zeroizing};
type Authentication = Hmac<Sha1>;
enum Cipher {
    A128(Box<ctr::Ctr128LE<aes::Aes128>>),
    A192(Box<ctr::Ctr128LE<aes::Aes192>>),
    A256(Box<ctr::Ctr128LE<aes::Aes256>>),
}
impl Cipher {
    fn new(key: &[u8]) -> Result<Self> {
        let mut iv = [0u8; 16];
        iv[0] = 1;
        match key.len() {
            16 => Ok(Self::A128(Box::new(
                ctr::Ctr128LE::<aes::Aes128>::new_from_slices(key, &iv)
                    .map_err(|_| Error::Malformed("AES key".into()))?,
            ))),
            24 => Ok(Self::A192(Box::new(
                ctr::Ctr128LE::<aes::Aes192>::new_from_slices(key, &iv)
                    .map_err(|_| Error::Malformed("AES key".into()))?,
            ))),
            32 => Ok(Self::A256(Box::new(
                ctr::Ctr128LE::<aes::Aes256>::new_from_slices(key, &iv)
                    .map_err(|_| Error::Malformed("AES key".into()))?,
            ))),
            _ => Err(Error::Unsupported("AES strength".into())),
        }
    }
    fn apply(&mut self, data: &mut [u8]) {
        match self {
            Self::A128(c) => c.apply_keystream(data),
            Self::A192(c) => c.apply_keystream(data),
            Self::A256(c) => c.apply_keystream(data),
        }
    }
}
fn derive(password: &[u8], salt: &[u8], key_size: usize) -> Result<Zeroizing<Vec<u8>>> {
    if password.len() > 1 << 20 {
        return Err(Error::ResourceLimit("password bytes"));
    }
    let mut derived = Zeroizing::new(vec![0u8; key_size * 2 + 2]);
    pbkdf2::pbkdf2::<Authentication>(password, salt, 1000, &mut derived)
        .map_err(|_| Error::Malformed("password derivation".into()))?;
    Ok(derived)
}
pub(crate) struct DecryptReader<R> {
    reader: R,
    cipher: Cipher,
}
impl<R: Read> Read for DecryptReader<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let n = self.reader.read(output)?;
        self.cipher.apply(&mut output[..n]);
        Ok(n)
    }
}
/// Authenticate ciphertext before returning a decoding stream; no unverified plaintext escapes.
pub(crate) fn decrypt<'a, R: Read + Seek>(
    reader: &'a mut R,
    offset: u64,
    size: u64,
    strength: u8,
    password: &[u8],
) -> Result<DecryptReader<std::io::Take<&'a mut R>>> {
    let key_size = match strength {
        1 => 16,
        2 => 24,
        3 => 32,
        _ => return Err(Error::Unsupported("WinZip AES strength".into())),
    };
    let salt_size = key_size / 2;
    let payload = size
        .checked_sub(salt_size as u64 + 12)
        .ok_or_else(|| Error::Malformed("truncated AES payload".into()))?;
    reader.seek(SeekFrom::Start(offset))?;
    let mut salt = vec![0u8; salt_size];
    reader.read_exact(&mut salt)?;
    let mut verifier = [0u8; 2];
    reader.read_exact(&mut verifier)?;
    let derived = derive(password, &salt, key_size)?;
    if verifier
        .ct_eq(&derived[key_size * 2..key_size * 2 + 2])
        .unwrap_u8()
        != 1
    {
        return Err(Error::Integrity("password verifier mismatch".into()));
    }
    let mut mac = Authentication::new_from_slice(&derived[key_size..key_size * 2])
        .map_err(|_| Error::Malformed("authentication key".into()))?;
    let mut remaining = payload;
    let mut buffer = [0u8; 65536];
    while remaining != 0 {
        let n = remaining.min(buffer.len() as u64) as usize;
        reader.read_exact(&mut buffer[..n])?;
        mac.update(&buffer[..n]);
        remaining -= n as u64;
    }
    let mut tag = [0u8; 10];
    reader.read_exact(&mut tag)?;
    let calculated = mac.finalize().into_bytes();
    if tag.ct_eq(&calculated[..10]).unwrap_u8() != 1 {
        return Err(Error::Integrity(
            "WinZip AES authentication mismatch".into(),
        ));
    }
    reader.seek(SeekFrom::Start(offset + salt_size as u64 + 2))?;
    let cipher = Cipher::new(&derived[..key_size])?;
    Ok(DecryptReader {
        reader: reader.take(payload),
        cipher,
    })
}
struct LegacyKeys([u32; 3]);
impl Drop for LegacyKeys {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}
impl LegacyKeys {
    fn new(password: &[u8]) -> Self {
        let mut keys = Self([0x12345678, 0x23456789, 0x34567890]);
        for byte in password {
            keys.update(*byte);
        }
        keys
    }
    fn update(&mut self, byte: u8) {
        self.0[0] = ms_compress::zlib::crc32::crc32(self.0[0] ^ u32::MAX, &[byte]) ^ u32::MAX;
        self.0[1] = self.0[1]
            .wrapping_add(self.0[0] & 255)
            .wrapping_mul(134775813)
            .wrapping_add(1);
        self.0[2] =
            ms_compress::zlib::crc32::crc32(self.0[2] ^ u32::MAX, &[(self.0[1] >> 24) as u8])
                ^ u32::MAX;
    }
    fn mask(&self) -> u8 {
        let temp = (self.0[2] & 65535) | 2;
        (temp.wrapping_mul(temp ^ 1) >> 8) as u8
    }
    fn decrypt(&mut self, byte: u8) -> u8 {
        let plain = byte ^ self.mask();
        self.update(plain);
        plain
    }
}
pub(crate) struct LegacyReader<R> {
    reader: R,
    keys: LegacyKeys,
}
impl<R: Read> Read for LegacyReader<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let n = self.reader.read(output)?;
        for byte in &mut output[..n] {
            *byte = self.keys.decrypt(*byte);
        }
        Ok(n)
    }
}
pub(crate) fn legacy_decrypt<R: Read>(
    reader: R,
    password: &[u8],
    check: u8,
) -> Result<LegacyReader<R>> {
    if password.len() > 1 << 20 {
        return Err(Error::ResourceLimit("password bytes"));
    }
    let mut reader = LegacyReader {
        reader,
        keys: LegacyKeys::new(password),
    };
    let mut header = [0u8; 12];
    reader.read_exact(&mut header)?;
    if header[11] != check {
        return Err(Error::Integrity("legacy password verifier mismatch".into()));
    }
    Ok(reader)
}
pub(crate) struct EncryptWriter<W> {
    writer: W,
    state: EncryptState,
}
enum EncryptState {
    Aes { cipher: Cipher, mac: Authentication },
    Legacy(LegacyKeys),
}
impl<W: std::io::Write> EncryptWriter<W> {
    pub(crate) fn new(
        mut writer: W,
        password: &[u8],
        random: &mut dyn crate::RandomSource,
        mode: crate::ZipEncryption,
        check: u8,
    ) -> Result<Self> {
        let state = match mode {
            crate::ZipEncryption::Aes256 => {
                let mut salt = [0; 16];
                random.fill(&mut salt)?;
                let derived = derive(password, &salt, 32)?;
                writer.write_all(&salt)?;
                writer.write_all(&derived[64..66])?;
                EncryptState::Aes {
                    cipher: Cipher::new(&derived[..32])?,
                    mac: Authentication::new_from_slice(&derived[32..64])
                        .map_err(|_| Error::Malformed("authentication key".into()))?,
                }
            }
            crate::ZipEncryption::ZipCrypto => {
                if password.len() > 1 << 20 {
                    return Err(Error::ResourceLimit("password bytes"));
                }
                let mut keys = LegacyKeys::new(password);
                let mut header = [0; 12];
                random.fill(&mut header[..11])?;
                header[11] = check;
                for byte in &mut header {
                    let plain = *byte;
                    *byte ^= keys.mask();
                    keys.update(plain);
                }
                writer.write_all(&header)?;
                EncryptState::Legacy(keys)
            }
        };
        Ok(Self { writer, state })
    }
    pub(crate) fn finish(mut self) -> Result<()> {
        if let EncryptState::Aes { mac, .. } = self.state {
            self.writer.write_all(&mac.finalize().into_bytes()[..10])?;
        }
        Ok(())
    }
}
impl<W: std::io::Write> std::io::Write for EncryptWriter<W> {
    fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
        let mut buffer = [0; 65536];
        let count = input.len().min(buffer.len());
        buffer[..count].copy_from_slice(&input[..count]);
        match &mut self.state {
            EncryptState::Aes { cipher, mac } => {
                cipher.apply(&mut buffer[..count]);
                mac.update(&buffer[..count]);
            }
            EncryptState::Legacy(keys) => {
                for byte in &mut buffer[..count] {
                    let plain = *byte;
                    *byte ^= keys.mask();
                    keys.update(plain);
                }
            }
        }
        self.writer.write_all(&buffer[..count])?;
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}
