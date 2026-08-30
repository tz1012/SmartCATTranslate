use crate::documents::types::DocumentError;
use std::{
    collections::{BTreeMap, HashSet},
    fs::File,
    io::{BufReader, Cursor, Read, Write},
    path::{Component, Path},
};
use zip::{read::ZipArchive, write::SimpleFileOptions, CompressionMethod, ZipWriter};

const MAX_ENTRIES: usize = 20_000;
const MAX_ENTRY: u64 = 256 * 1024 * 1024;
const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
const MAX_COMPRESSION_RATIO: u64 = 200;
const PREVIEW_MAX_ENTRY: u64 = 16 * 1024 * 1024;
const PREVIEW_MAX_TOTAL: u64 = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct Entry {
    pub bytes: Vec<u8>,
    pub method: CompressionMethod,
    pub mode: Option<u32>,
}
pub struct OoxmlPackage {
    pub entries: BTreeMap<String, Entry>,
}

impl OoxmlPackage {
    pub fn open(bytes: &[u8]) -> Result<Self, DocumentError> {
        let mut zip =
            ZipArchive::new(Cursor::new(bytes)).map_err(|_| DocumentError::InvalidPackage)?;
        if zip.len() > MAX_ENTRIES {
            return Err(DocumentError::InvalidPackage);
        }
        let mut total = 0u64;
        let mut entries = BTreeMap::new();
        for index in 0..zip.len() {
            let mut file = zip
                .by_index(index)
                .map_err(|_| DocumentError::InvalidPackage)?;
            let name = file.name().replace('\\', "/");
            if file.encrypted()
                || !safe_name(&name)
                || file.size() > MAX_ENTRY
                || suspicious_compression(file.size(), file.compressed_size())
            {
                return Err(DocumentError::InvalidPackage);
            }
            total = total
                .checked_add(file.size())
                .ok_or(DocumentError::InvalidPackage)?;
            if total > MAX_TOTAL || entries.contains_key(&name) {
                return Err(DocumentError::InvalidPackage);
            }
            let method = file.compression();
            let mode = file.unix_mode();
            let mut data = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut data)
                .map_err(|_| DocumentError::InvalidPackage)?;
            entries.insert(
                name,
                Entry {
                    bytes: data,
                    method,
                    mode,
                },
            );
        }
        if !entries.contains_key("[Content_Types].xml") {
            return Err(DocumentError::InvalidPackage);
        }
        Ok(Self { entries })
    }
    pub fn read(&self, name: &str) -> Option<&[u8]> {
        self.entries.get(name).map(|v| v.bytes.as_slice())
    }
    pub fn replace(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), DocumentError> {
        let entry = self
            .entries
            .get_mut(name)
            .ok_or(DocumentError::InvalidPackage)?;
        entry.bytes = bytes;
        Ok(())
    }
    pub fn write(&self) -> Result<Vec<u8>, DocumentError> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(cursor);
        for (name, entry) in &self.entries {
            let mut options = SimpleFileOptions::default().compression_method(entry.method);
            if let Some(mode) = entry.mode {
                options = options.unix_permissions(mode);
            }
            if name.ends_with('/') {
                writer
                    .add_directory(name, options)
                    .map_err(|_| DocumentError::Io)?;
            } else {
                writer
                    .start_file(name, options)
                    .map_err(|_| DocumentError::Io)?;
                writer
                    .write_all(&entry.bytes)
                    .map_err(|_| DocumentError::Io)?;
            }
        }
        writer
            .finish()
            .map(|v| v.into_inner())
            .map_err(|_| DocumentError::Io)
    }
}

pub struct PreviewPackage {
    archive: ZipArchive<BufReader<File>>,
    read_total: u64,
}

impl PreviewPackage {
    pub fn open(path: &Path) -> Result<Self, DocumentError> {
        let file = File::open(path).map_err(|_| DocumentError::Io)?;
        let mut archive =
            ZipArchive::new(BufReader::new(file)).map_err(|_| DocumentError::InvalidPackage)?;
        if archive.len() > MAX_ENTRIES {
            return Err(DocumentError::InvalidPackage);
        }
        let mut names = HashSet::with_capacity(archive.len());
        let mut declared_total = 0u64;
        for index in 0..archive.len() {
            let file = archive
                .by_index(index)
                .map_err(|_| DocumentError::InvalidPackage)?;
            let name = file.name().replace('\\', "/");
            if file.encrypted()
                || !safe_name(&name)
                || file.size() > MAX_ENTRY
                || suspicious_compression(file.size(), file.compressed_size())
                || !names.insert(name)
            {
                return Err(DocumentError::InvalidPackage);
            }
            declared_total = declared_total
                .checked_add(file.size())
                .ok_or(DocumentError::InvalidPackage)?;
            if declared_total > MAX_TOTAL {
                return Err(DocumentError::InvalidPackage);
            }
        }
        if !names.contains("[Content_Types].xml") {
            return Err(DocumentError::InvalidPackage);
        }
        Ok(Self {
            archive,
            read_total: 0,
        })
    }

    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, DocumentError> {
        if !safe_name(name) {
            return Err(DocumentError::InvalidPackage);
        }
        let mut file = self
            .archive
            .by_name(name)
            .map_err(|_| DocumentError::Unsupported)?;
        if file.encrypted()
            || file.size() > PREVIEW_MAX_ENTRY
            || suspicious_compression(file.size(), file.compressed_size())
            || self.read_total.saturating_add(file.size()) > PREVIEW_MAX_TOTAL
        {
            return Err(DocumentError::PreviewLimitExceeded);
        }
        let expected = file.size();
        let mut bytes = Vec::with_capacity(expected.min(PREVIEW_MAX_ENTRY) as usize);
        (&mut file)
            .take(PREVIEW_MAX_ENTRY + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| DocumentError::InvalidPackage)?;
        if bytes.len() as u64 != expected || bytes.len() as u64 > PREVIEW_MAX_ENTRY {
            return Err(DocumentError::PreviewLimitExceeded);
        }
        self.read_total = self
            .read_total
            .checked_add(bytes.len() as u64)
            .ok_or(DocumentError::PreviewLimitExceeded)?;
        Ok(bytes)
    }

    pub fn read_optional(&mut self, name: &str) -> Result<Option<Vec<u8>>, DocumentError> {
        match self.read(name) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(DocumentError::Unsupported) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn suspicious_compression(size: u64, compressed_size: u64) -> bool {
    size > 1024 * 1024
        && (compressed_size == 0 || size / compressed_size.max(1) > MAX_COMPRESSION_RATIO)
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && !name.contains(':')
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
