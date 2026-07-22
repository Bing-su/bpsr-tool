use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Component, Utf8Path, Utf8PathBuf};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry {
    pub key: u32,
    pub offset: u32,
    pub index: u16,
    pub length: u32,
}

pub(crate) struct Package {
    root: Utf8PathBuf,
    entries: BTreeMap<u32, Entry>,
    patches: BTreeMap<u32, Utf8PathBuf>,
}

impl Package {
    pub fn open(meta: &Utf8Path) -> Result<Self> {
        let root = meta
            .parent()
            .context("meta.pkg has no parent directory")?
            .to_owned();
        let bytes = fs::read(meta)?;
        let mut cursor = Cursor::new(&bytes);
        cursor.skip(4 * 3 + 8 + 4)?;
        let file_count = cursor.i16()?;
        if file_count < 0 {
            bail!("negative container count");
        }
        cursor.skip(
            usize::from(file_count as u16)
                .checked_mul(16)
                .context("header overflow")?,
        )?;
        let mut entries = BTreeMap::new();
        for _ in 0..2 {
            let count = cursor.i32()?;
            if count < 0 {
                bail!("negative entry count");
            }
            for _ in 0..count {
                let entry = Entry {
                    key: cursor.u32()?,
                    index: {
                        cursor.u8()?;
                        cursor.u16()?
                    },
                    offset: nonnegative(cursor.i32()?, "entry offset")?,
                    length: nonnegative(cursor.i32()?, "entry length")?,
                };
                entries.insert(entry.key, entry);
            }
        }

        let mut patches = BTreeMap::new();
        let patch_dir = root.join("Patch");
        if patch_dir.is_dir() {
            for item in patch_dir.read_dir_utf8()? {
                let item = item?;
                let path = item.path();
                if !item.file_type()?.is_file() {
                    continue;
                }
                if let Some(key) = path.file_stem().and_then(|s| s.parse().ok()) {
                    let length =
                        u32::try_from(path.metadata()?.len()).context("patch is too large")?;
                    patches.insert(key, path.to_owned());
                    entries.entry(key).or_insert(Entry {
                        key,
                        offset: 0,
                        index: 0,
                        length,
                    });
                }
            }
        }
        Ok(Self {
            root,
            entries,
            patches,
        })
    }

    pub fn entries(&self) -> &BTreeMap<u32, Entry> {
        &self.entries
    }

    pub fn read_by_key(&self, key: u32) -> Result<Option<Vec<u8>>> {
        self.entries
            .get(&key)
            .map(|entry| self.read(entry))
            .transpose()
    }

    pub fn read(&self, entry: &Entry) -> Result<Vec<u8>> {
        if let Some(path) = self.patches.get(&entry.key) {
            return fs::read(path).with_context(|| format!("failed to read {path}"));
        }
        let path = self.root.join(format!("m{}.pkg", entry.index));
        let mut file = File::open(&path).with_context(|| format!("missing container {path}"))?;
        let end = u64::from(entry.offset) + u64::from(entry.length);
        if end > file.metadata()?.len() {
            bail!("entry {} lies outside {path}", entry.key);
        }
        file.seek(SeekFrom::Start(u64::from(entry.offset)))?;
        let mut data = vec![0; entry.length as usize];
        file.read_exact(&mut data)?;
        Ok(data)
    }
}

pub(crate) fn write_lua(root: &Utf8Path, key: u32, data: &[u8]) -> Result<()> {
    const OFFSET: usize = 0x22;
    let length = usize::from(*data.get(OFFSET).context("Lua path length is missing")?);
    if length == 0 || OFFSET + length > data.len() {
        bail!("Lua entry {key} has an invalid embedded path");
    }
    let raw = std::str::from_utf8(&data[OFFSET + 1..OFFSET + length])?;
    let relative = raw
        .split_once("Standalone/container/lua/")
        .map_or(raw, |(_, path)| path);
    let mut safe = Utf8PathBuf::new();
    for component in Utf8Path::new(relative).components() {
        match component {
            Utf8Component::Normal(part) => safe.push(part),
            Utf8Component::CurDir => {}
            _ => bail!("Lua entry {key} contains an unsafe path"),
        }
    }
    if safe.as_os_str().is_empty() {
        bail!("Lua entry {key} has an empty path");
    }
    let path = root.join("Lua").join(format!("{safe}c"));
    fs::create_dir_all(path.parent().context("Lua output has no parent")?)?;
    let mut output = data.to_vec();
    if output.len() > 5 {
        output[5] = 0;
    }
    fs::write(path, output)?;
    Ok(())
}

fn nonnegative(value: i32, name: &str) -> Result<u32> {
    u32::try_from(value).with_context(|| format!("negative {name}"))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        let end = self
            .position
            .checked_add(N)
            .context("input offset overflow")?;
        let data: [u8; N] = self
            .bytes
            .get(self.position..end)
            .context("truncated package index")?
            .try_into()
            .unwrap();
        self.position = end;
        Ok(data)
    }
    fn skip(&mut self, count: usize) -> Result<()> {
        self.take_slice(count).map(drop)
    }
    fn take_slice(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(count)
            .context("input offset overflow")?;
        let data = self
            .bytes
            .get(self.position..end)
            .context("truncated package index")?;
        self.position = end;
        Ok(data)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.take()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(bytes: &mut Vec<u8>, key: u32, index: u16, offset: i32, length: i32) {
        bytes.extend(key.to_le_bytes());
        bytes.push(0);
        bytes.extend(index.to_le_bytes());
        bytes.extend(offset.to_le_bytes());
        bytes.extend(length.to_le_bytes());
    }

    #[test]
    fn reads_both_indexes_and_prefers_patch() {
        let root = Utf8PathBuf::try_from(std::env::temp_dir())
            .unwrap()
            .join(format!("bpsr-package-{}", std::process::id()));
        fs::create_dir_all(root.join("Patch")).unwrap();
        let mut meta = vec![0; 24];
        meta.extend(0_i16.to_le_bytes());
        meta.extend(1_i32.to_le_bytes());
        entry(&mut meta, 10, 0, 1, 2);
        meta.extend(1_i32.to_le_bytes());
        entry(&mut meta, 20, 1, 0, 3);
        fs::write(root.join("meta.pkg"), meta).unwrap();
        fs::write(root.join("m0.pkg"), b"xaby").unwrap();
        fs::write(root.join("m1.pkg"), b"old").unwrap();
        fs::write(root.join("Patch/20.bin"), b"new").unwrap();

        let package = Package::open(&root.join("meta.pkg")).unwrap();
        assert_eq!(package.read_by_key(10).unwrap().unwrap(), b"ab");
        assert_eq!(package.read_by_key(20).unwrap().unwrap(), b"new");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_truncated_index() {
        let path = Utf8PathBuf::try_from(std::env::temp_dir())
            .unwrap()
            .join(format!("bpsr-truncated-{}.pkg", std::process::id()));
        fs::write(&path, [0; 4]).unwrap();
        assert!(Package::open(&path).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_lua_path_traversal() {
        let mut data = vec![0; 0x30];
        data[0x22] = 10;
        data[0x23..0x2c].copy_from_slice(b"../bad.lu");
        assert!(write_lua(Utf8Path::new("unused"), 1, &data).is_err());
    }
}
