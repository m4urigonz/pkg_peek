//! Read-only access to the metadata of PS4 PKG files.
//!
//! ```no_run
//! let file = std::fs::File::open("game.pkg")?;
//! let mut pkg = pkg_core::Ps4Pkg::open(std::io::BufReader::new(file))?;
//! let info = pkg_core::PkgInfo::read(&mut pkg)?;
//! println!("{} [{}]", info.title, info.title_id);
//! # Ok::<(), pkg_core::Error>(())
//! ```

pub mod artwork;
pub mod info;
pub mod pkg;
pub mod sfo;
pub mod siblings;
pub mod trp;

pub use artwork::{Artwork, IconSource};
pub use info::{Category, PkgInfo, Region};
pub use pkg::{ContentType, Ps4Pkg, is_ps4_pkg};

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    NotPs4Pkg,
    Truncated,
    Corrupt(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "Error de lectura: {e}"),
            Self::NotPs4Pkg => f.write_str("No es un PKG de PS4"),
            Self::Truncated => f.write_str("El PKG está truncado"),
            Self::Corrupt(what) => write!(f, "PKG dañado ({what})"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            Self::Truncated
        } else {
            Self::Io(e)
        }
    }
}
