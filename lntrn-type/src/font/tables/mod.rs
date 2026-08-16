//! Fixed-layout sfnt tables, split by concern:
//! - [`metrics`] — `head`, `hhea`, `maxp`, `hmtx` (sizing + advances)
//! - [`name`] — family / typographic names
//! - [`os2`] — `OS/2` weight/width/style classification + `post` fixed-pitch

pub(crate) mod metrics;
pub(crate) mod name;
pub(crate) mod os2;

pub(crate) use metrics::{hmtx_advance, parse_head, parse_hhea, parse_maxp};
