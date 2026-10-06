//! The pictures a document holds. A picture is known by a number made
//! from its bytes, so the same picture put in twice is kept once, and one
//! copied to another document is the same picture there.

use std::collections::BTreeMap;
use std::sync::Arc;

use lntrn_image::Image;

/// A picture: the file it came as, kept to be saved as it was, and its
/// pixels.
#[derive(Debug)]
pub struct Picture {
    pub bytes: Vec<u8>,
    pub image: Image,
}

impl Picture {
    /// Its number: FNV-1a over the bytes.
    fn id_of(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)).max(1)
    }

    /// What kind of file it is, as an extension.
    pub fn extension(&self) -> &'static str {
        lntrn_image::Format::sniff(&self.bytes).map_or("png", lntrn_image::Format::extension)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Pictures(BTreeMap<u64, Arc<Picture>>);

impl PartialEq for Pictures {
    fn eq(&self, other: &Self) -> bool {
        self.0.keys().eq(other.0.keys())
    }
}

impl Pictures {
    /// Take in a picture file. Returns its number, or why it could not
    /// be read.
    pub fn add(&mut self, bytes: Vec<u8>) -> Result<u64, String> {
        let id = Picture::id_of(&bytes);
        if let std::collections::btree_map::Entry::Vacant(room) = self.0.entry(id) {
            let image = lntrn_image::decode(&bytes).map_err(|e| e.to_string())?;
            if image.width == 0 || image.height == 0 {
                return Err("the picture is empty".to_owned());
            }
            room.insert(Arc::new(Picture { bytes, image }));
        }
        Ok(id)
    }

    /// Take in pixels that came as no file (a paste): kept as a PNG.
    pub fn add_image(&mut self, image: &Image) -> Result<u64, String> {
        self.add(lntrn_image::encode_png(image))
    }

    pub fn get(&self, id: u64) -> Option<&Arc<Picture>> {
        self.0.get(&id)
    }

    /// Share a picture another document has.
    pub fn share(&mut self, id: u64, picture: &Arc<Picture>) {
        self.0.entry(id).or_insert_with(|| picture.clone());
    }

    #[cfg(test)]
    pub fn iter(&self) -> impl Iterator<Item = (u64, &Arc<Picture>)> {
        self.0.iter().map(|(id, p)| (*id, p))
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_kept_once_and_known_by_what_it_is() {
        let mut a = Pictures::default();
        let red = lntrn_image::encode_png(&Image::solid(4, 2, [255, 0, 0, 255]));
        let id = a.add(red.clone()).unwrap();
        assert_eq!((a.add(red).unwrap(), a.iter().count()), (id, 1));
        let other = a.add_image(&Image::solid(1, 1, [0, 0, 255, 255])).unwrap();
        assert_ne!(id, other);
        let p = a.get(id).unwrap();
        assert_eq!((p.image.width, p.image.height, p.extension()), (4, 2, "png"));
        assert!(a.add(b"not a picture".to_vec()).is_err());
        // The same number in another document.
        let mut b = Pictures::default();
        b.share(id, a.get(id).unwrap());
        assert!(b.get(id).is_some() && b != a);
        b.share(other, a.get(other).unwrap());
        assert_eq!(a, b);
    }
}
