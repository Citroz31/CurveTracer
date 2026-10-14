//! Minimal ZIP writer ("stored" entries, no compression). Enough for Office
//! Open XML packages: the embedded PNG images are already compressed.

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

struct Entry {
    name: String,
    crc: u32,
    size: u32,
    offset: u32,
}

#[derive(Default)]
pub struct ZipWriter {
    buf: Vec<u8>,
    entries: Vec<Entry>,
}

fn u16le(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn u32le(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (46 << 9) | (1 << 5) | 1; // 2026-01-01

impl ZipWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, name: &str, data: &[u8]) {
        let crc = crc32(data);
        let offset = self.buf.len() as u32;
        let b = &mut self.buf;
        u32le(b, 0x0403_4b50);
        u16le(b, 20); // version needed
        u16le(b, 0x0800); // UTF-8 names
        u16le(b, 0); // stored
        u16le(b, DOS_TIME);
        u16le(b, DOS_DATE);
        u32le(b, crc);
        u32le(b, data.len() as u32);
        u32le(b, data.len() as u32);
        u16le(b, name.len() as u16);
        u16le(b, 0);
        b.extend_from_slice(name.as_bytes());
        b.extend_from_slice(data);
        self.entries.push(Entry {
            name: name.to_owned(),
            crc,
            size: data.len() as u32,
            offset,
        });
    }

    pub fn finish(mut self) -> Vec<u8> {
        let cd_start = self.buf.len() as u32;
        let mut cd = Vec::new();
        for e in &self.entries {
            u32le(&mut cd, 0x0201_4b50);
            u16le(&mut cd, 20); // made by
            u16le(&mut cd, 20); // needed
            u16le(&mut cd, 0x0800);
            u16le(&mut cd, 0);
            u16le(&mut cd, DOS_TIME);
            u16le(&mut cd, DOS_DATE);
            u32le(&mut cd, e.crc);
            u32le(&mut cd, e.size);
            u32le(&mut cd, e.size);
            u16le(&mut cd, e.name.len() as u16);
            u16le(&mut cd, 0); // extra
            u16le(&mut cd, 0); // comment
            u16le(&mut cd, 0); // disk
            u16le(&mut cd, 0); // internal attributes
            u32le(&mut cd, 0); // external attributes
            u32le(&mut cd, e.offset);
            cd.extend_from_slice(e.name.as_bytes());
        }
        let cd_size = cd.len() as u32;
        self.buf.extend_from_slice(&cd);
        let n = self.entries.len() as u16;
        let b = &mut self.buf;
        u32le(b, 0x0605_4b50);
        u16le(b, 0);
        u16le(b, 0);
        u16le(b, n);
        u16le(b, n);
        u32le(b, cd_size);
        u32le(b, cd_start);
        u16le(b, 0);
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn archive_layout() {
        let mut z = ZipWriter::new();
        z.add("a.txt", b"hello");
        z.add("dir/b.txt", b"world!");
        let bytes = z.finish();
        assert_eq!(&bytes[..4], &[0x50, 0x4b, 0x03, 0x04]);
        let eocd = &bytes[bytes.len() - 22..];
        assert_eq!(&eocd[..4], &[0x50, 0x4b, 0x05, 0x06]);
        assert_eq!(u16::from_le_bytes([eocd[10], eocd[11]]), 2);
    }
}
