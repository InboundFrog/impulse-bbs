//! Archive directory listings (ZIPVIEWU.PAS, LZHVIEW.PAS), read natively
//! from the headers: ZIP central directory, LHA level 0/1, ARJ.

pub struct Entry {
    pub name: String,
    pub method: String,
    pub size: u32,
    pub csize: u32,
    /// MS-DOS packed date (high word) and time (low word)
    pub dos: u32,
}

fn u16_at(b: &[u8], p: usize) -> Option<u32> {
    Some(u16::from_le_bytes(b.get(p..p + 2)?.try_into().ok()?).into())
}

fn u32_at(b: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(p..p + 4)?.try_into().ok()?))
}

/// None if `b` isn't an archive we can read.
pub fn list(b: &[u8]) -> Option<Vec<Entry>> {
    if b.starts_with(b"PK") {
        zip(b)
    } else if b.starts_with(&[0x60, 0xEA]) {
        arj(b)
    } else if b.get(2..4) == Some(b"-l") {
        lzh(b)
    } else {
        None
    }
}

fn zip(b: &[u8]) -> Option<Vec<Entry>> {
    let end = (0..b.len().saturating_sub(21)).rev().find(|&i| b[i..].starts_with(b"PK\x05\x06"))?;
    let (n, mut p) = (u16_at(b, end + 10)?, u32_at(b, end + 16)? as usize);
    let mut v = vec![];
    for _ in 0..n {
        if u32_at(b, p)? != 0x0201_4b50 {
            return None;
        }
        let nl = u16_at(b, p + 28)? as usize;
        let m = u16_at(b, p + 10)?;
        v.push(Entry {
            name: String::from_utf8_lossy(b.get(p + 46..p + 46 + nl)?).into(),
            method: ["Stored", "Shrunk", "Reduce1", "Reduce2", "Reduce3", "Reduce4", "Implode", "NotSure", "Deflate", "Deflat64"]
                .get(m as usize)
                .map_or_else(|| format!("#{m}"), |s| s.to_string()),
            size: u32_at(b, p + 24)?,
            csize: u32_at(b, p + 20)?,
            dos: u16_at(b, p + 14)? << 16 | u16_at(b, p + 12)?,
        });
        p += 46 + nl + u16_at(b, p + 30)? as usize + u16_at(b, p + 32)? as usize;
    }
    Some(v)
}

fn lzh(b: &[u8]) -> Option<Vec<Entry>> {
    let (mut p, mut v) = (0, vec![]);
    while b.get(p).is_some_and(|&h| h > 0) {
        let csize = u32_at(b, p + 7)?;
        let level = *b.get(p + 20)?;
        let (name, dos, hs) = if level == 2 {
            // header size is a word; the name is extended header 1; time is Unix
            let (mut name, mut q, mut next) = (&b[0..0], p + 26, u16_at(b, p + 24)? as usize);
            while next > 2 {
                if *b.get(q)? == 1 {
                    name = b.get(q + 1..q + next - 2)?;
                }
                (q, next) = (q + next, u16_at(b, q + next - 2)? as usize);
            }
            (name, unix_dos(u32_at(b, p + 15)?), u16_at(b, p)? as usize)
        } else {
            let nl = *b.get(p + 21)? as usize;
            (b.get(p + 22..p + 22 + nl)?, u32_at(b, p + 15)?, b[p] as usize + 2)
        };
        v.push(Entry {
            name: String::from_utf8_lossy(name).into(),
            method: String::from_utf8_lossy(b.get(p + 3..p + 6)?).into(),
            size: u32_at(b, p + 11)?,
            csize,
            dos,
        });
        p += hs + csize as usize;
    }
    Some(v)
}

/// Unix time as an MS-DOS packed date and time (local time).
fn unix_dos(t: u32) -> u32 {
    use chrono::{Datelike, Timelike};
    let Some(d) = chrono::DateTime::from_timestamp(t.into(), 0) else { return 0 };
    let d = d.with_timezone(&chrono::Local);
    let date = ((d.year() - 1980).max(0) as u32) << 9 | d.month() << 5 | d.day();
    date << 16 | d.hour() << 11 | d.minute() << 5 | (d.second() / 2)
}

fn arj(b: &[u8]) -> Option<Vec<Entry>> {
    let (mut p, mut v, mut main) = (0, vec![], true);
    loop {
        if u16_at(b, p)? != 0xEA60 {
            return None;
        }
        let hs = u16_at(b, p + 2)? as usize;
        if hs == 0 {
            return Some(v);
        }
        let h = b.get(p + 4..p + 4 + hs)?;
        p += 4 + hs + 4;
        while let Some(ext) = u16_at(b, p).filter(|&e| e > 0) {
            p += 2 + ext as usize + 4;
        }
        p += 2;
        if main {
            main = false;
            continue;
        }
        let first = *h.first()? as usize;
        let name = h.get(first..)?.split(|&c| c == 0).next()?;
        let csize = u32_at(h, 12)?;
        v.push(Entry {
            name: String::from_utf8_lossy(name).into(),
            method: ["Stored", "Most", "Method2", "Method3", "Fastest"].get(*h.get(5)? as usize).unwrap_or(&"Unknown").to_string(),
            size: u32_at(h, 16)?,
            csize,
            dos: u32_at(h, 8)?,
        });
        p += csize as usize;
    }
}

/// `MM/DD/YY` and `HH:MMa` like LZHVIEW's FDT.
pub fn dos_dt(d: u32) -> (String, String) {
    let (date, time) = (d >> 16, d & 0xFFFF);
    let h = time >> 11;
    let ap = if h < 12 { 'a' } else { 'p' };
    let h12 = if h > 12 { h - 12 } else { h };
    (format!("{:02}/{:02}/{:02}", date >> 5 & 15, date & 31, (1980 + (date >> 9)) % 100), format!("{h12:02}:{:02}{ap}", time >> 5 & 63))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers() {
        // 1994-03-05 14:30 in DOS form
        let dos: u32 = ((14 << 9 | 3 << 5 | 5) << 16) | (14 << 11 | 30 << 5);
        assert_eq!(dos_dt(dos), ("03/05/94".into(), "02:30p".into()));

        // level 2: word header size, name in extended header 1
        let mut l2 = vec![0u8; 26];
        l2[2..7].copy_from_slice(b"-lh0-");
        l2[7..11].copy_from_slice(&2u32.to_le_bytes());
        l2[11..15].copy_from_slice(&2u32.to_le_bytes());
        l2[20] = 2;
        l2[24] = 8; // first extended header: type, "C.TXT", next size
        l2.extend([1]);
        l2.extend(b"C.TXT");
        l2.extend([0, 0]);
        let hs = l2.len() as u16;
        l2[0..2].copy_from_slice(&hs.to_le_bytes());
        l2.extend(b"ok\0");
        let v = list(&l2).unwrap();
        assert_eq!((v.len(), v[0].name.as_str(), v[0].size), (1, "C.TXT", 2));

        let mut lzh = vec![0u8; 22];
        lzh[0] = 22 + 5 + 2 - 2; // header size excludes the first two bytes
        lzh[2..7].copy_from_slice(b"-lh5-");
        lzh[7..11].copy_from_slice(&3u32.to_le_bytes());
        lzh[11..15].copy_from_slice(&10u32.to_le_bytes());
        lzh[15..19].copy_from_slice(&dos.to_le_bytes());
        lzh[21] = 5;
        lzh.extend(b"A.TXT\0\0xyz\0");
        let v = list(&lzh).unwrap();
        assert_eq!((v.len(), v[0].name.as_str(), v[0].method.as_str(), v[0].size, v[0].csize), (1, "A.TXT", "lh5", 10, 3));

        let arj_hdr = |name: &[u8], csize: u32| {
            let mut h = vec![30u8, 0, 0, 0, 0, 1, 0, 0];
            h.extend(dos.to_le_bytes());
            h.extend(csize.to_le_bytes());
            h.extend(7u32.to_le_bytes());
            h.resize(30, 0);
            h.extend(name);
            h.extend([0, 0]);
            let mut r = vec![0x60, 0xEA];
            r.extend((h.len() as u16).to_le_bytes());
            r.extend(h);
            r.extend([0; 6]); // crc, no extended header
            r.extend(vec![b'z'; csize as usize]);
            r
        };
        let mut arj = arj_hdr(b"MAIN.ARJ", 0);
        arj.extend(arj_hdr(b"B.DOC", 4));
        arj.extend([0x60, 0xEA, 0, 0]);
        let v = list(&arj).unwrap();
        assert_eq!((v.len(), v[0].name.as_str(), v[0].method.as_str(), v[0].size, v[0].csize), (1, "B.DOC", "Most", 7, 4));
        assert!(list(b"hello").is_none());
    }
}
