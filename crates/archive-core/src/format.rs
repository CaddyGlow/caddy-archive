use crate::{Error, Format, Result};

impl std::str::FromStr for Format {
    type Err = Error;

    /// Parse a format name or common suffix, independently of enabled backends.
    fn from_str(name: &str) -> Result<Self> {
        Ok(match name.to_ascii_lowercase().as_str() {
            "zip" => Self::Zip,
            "tar" => Self::Tar,
            "tar.gz" | "tgz" => Self::TarGzip,
            "cab" => Self::Cab,
            "7z" => Self::SevenZip,
            "xz" => Self::Xz,
            "tar.xz" | "txz" => Self::TarXz,
            "wim" | "esd" => Self::Wim,
            "iso" => Self::Iso,
            "udf" => Self::Udf,
            "appx" => Self::Appx,
            "msix" => Self::Msix,
            "msi" => Self::Msi,
            "gz" | "gzip" => Self::Gzip,
            "zlib" => Self::Zlib,
            "lzma" => Self::Lzma,
            "deflate" => Self::Deflate,
            "bz2" | "bzip2" => Self::Bzip2,
            "br" | "brotli" => Self::Brotli,
            "tar.bz2" | "tbz2" | "tbz" => Self::TarBzip2,
            "tar.br" => Self::TarBrotli,
            _ => return Err(Error::Unsupported(format!("unknown format {name:?}"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_are_case_insensitive_and_unknown_names_fail() {
        for (name, expected) in [
            ("TGZ", Format::TarGzip),
            ("txz", Format::TarXz),
            ("tbz", Format::TarBzip2),
            ("BZ2", Format::Bzip2),
            ("esd", Format::Wim),
        ] {
            assert_eq!(name.parse::<Format>().unwrap(), expected);
        }
        assert!("unknown".parse::<Format>().is_err());
    }
}
