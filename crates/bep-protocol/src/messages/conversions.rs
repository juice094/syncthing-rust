//! Conversions between BEP wire types and syncthing_core types

use super::{
    FileInfoType, Index, IndexUpdate, WireBlockInfo, WireCounter, WireFileInfo, WireVector,
};

impl From<syncthing_core::types::Vector> for WireVector {
    fn from(v: syncthing_core::types::Vector) -> Self {
        let counters = v
            .counters
            .into_iter()
            .map(|(id, value)| WireCounter { id, value })
            .collect();
        Self { counters }
    }
}

impl From<WireVector> for syncthing_core::types::Vector {
    fn from(v: WireVector) -> Self {
        let counters = v.counters.into_iter().map(|c| (c.id, c.value)).collect();
        Self { counters }
    }
}

impl From<syncthing_core::types::BlockInfo> for WireBlockInfo {
    fn from(b: syncthing_core::types::BlockInfo) -> Self {
        Self {
            offset: b.offset,
            size: b.size,
            hash: b.hash,
        }
    }
}

impl From<WireBlockInfo> for syncthing_core::types::BlockInfo {
    fn from(b: WireBlockInfo) -> Self {
        Self {
            offset: b.offset,
            size: b.size,
            hash: b.hash,
        }
    }
}

impl From<syncthing_core::types::FileInfo> for WireFileInfo {
    fn from(f: syncthing_core::types::FileInfo) -> Self {
        Self {
            name: f.name,
            r#type: match f.file_type {
                syncthing_core::types::FileType::Directory => FileInfoType::Directory as i32,
                syncthing_core::types::FileType::Symlink => FileInfoType::Symlink as i32,
                _ => FileInfoType::File as i32,
            },
            size: f.size,
            permissions: f.permissions,
            modified_s: f.modified_s,
            deleted: f.deleted.unwrap_or(false),
            invalid: false,
            no_permissions: f.no_permissions.unwrap_or(false),
            version: Some(f.version.into()),
            sequence: f.sequence as i64,
            modified_ns: f.modified_ns,
            modified_by: f.modified_by.unwrap_or(0),
            block_size: f.block_size,
            platform: None, // TODO: BEP protocol extension — platform data
            blocks: f.blocks.into_iter().map(Into::into).collect(),
            symlink_target: f.symlink_target.unwrap_or_default().into_bytes(),
            blocks_hash: f.blocks_hash.unwrap_or_default(),
            encrypted: Vec::new(), // TODO: BEP protocol extension — encrypted data
            previous_blocks_hash: Vec::new(), // TODO: BEP protocol extension — previous blocks hash
        }
    }
}

impl From<WireFileInfo> for syncthing_core::types::FileInfo {
    fn from(f: WireFileInfo) -> Self {
        Self {
            name: f.name,
            file_type: match f.r#type {
                x if x == FileInfoType::Directory as i32 => {
                    syncthing_core::types::FileType::Directory
                }
                x if x == FileInfoType::Symlink as i32 => syncthing_core::types::FileType::Symlink,
                _ => syncthing_core::types::FileType::File,
            },
            size: f.size,
            permissions: f.permissions,
            modified_s: f.modified_s,
            modified_ns: f.modified_ns,
            version: f.version.map(Into::into).unwrap_or_default(),
            sequence: f.sequence as u64,
            block_size: f.block_size,
            blocks: f.blocks.into_iter().map(Into::into).collect(),
            symlink_target: if f.symlink_target.is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(&f.symlink_target).to_string())
            },
            deleted: Some(f.deleted),
            modified_by: if f.modified_by == 0 {
                None
            } else {
                Some(f.modified_by)
            },
            blocks_hash: if f.blocks_hash.is_empty() {
                None
            } else {
                Some(f.blocks_hash)
            },
            no_permissions: if f.no_permissions { Some(true) } else { None },
            base_version: None,
        }
    }
}

impl From<syncthing_core::types::Index> for Index {
    fn from(idx: syncthing_core::types::Index) -> Self {
        let last_sequence = idx
            .files
            .iter()
            .map(|f| f.sequence)
            .max()
            .unwrap_or(0)
            .min(i64::MAX as u64) as i64;
        Self {
            folder: idx.folder,
            files: idx.files.into_iter().map(Into::into).collect(),
            last_sequence,
        }
    }
}

// 引用版本：避免 send_index() 中的 .clone() + .into() 双重分配
impl From<&syncthing_core::types::Index> for Index {
    fn from(idx: &syncthing_core::types::Index) -> Self {
        let last_sequence = idx
            .files
            .iter()
            .map(|f| f.sequence)
            .max()
            .unwrap_or(0)
            .min(i64::MAX as u64) as i64;
        Self {
            folder: idx.folder.clone(),
            files: idx.files.iter().map(|f| f.clone().into()).collect(),
            last_sequence,
        }
    }
}

impl From<Index> for syncthing_core::types::Index {
    fn from(idx: Index) -> Self {
        Self {
            folder: idx.folder,
            files: idx.files.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<syncthing_core::types::IndexUpdate> for IndexUpdate {
    fn from(upd: syncthing_core::types::IndexUpdate) -> Self {
        let sequences: Vec<u64> = upd.files.iter().map(|f| f.sequence).collect();
        let last_sequence = sequences.iter().copied().max().unwrap_or(0);
        let prev_sequence = sequences.iter().copied().min().unwrap_or(0);
        let prev_sequence = if prev_sequence > 0 {
            prev_sequence - 1
        } else {
            0
        };
        Self {
            folder: upd.folder,
            files: upd.files.into_iter().map(Into::into).collect(),
            last_sequence: last_sequence.min(i64::MAX as u64) as i64,
            prev_sequence: prev_sequence.min(i64::MAX as u64) as i64,
        }
    }
}

// 引用版本：避免 send_index_update() 中的 .clone() + .into() 双重分配
impl From<&syncthing_core::types::IndexUpdate> for IndexUpdate {
    fn from(upd: &syncthing_core::types::IndexUpdate) -> Self {
        let sequences: Vec<u64> = upd.files.iter().map(|f| f.sequence).collect();
        let last_sequence = sequences.iter().copied().max().unwrap_or(0);
        let prev_sequence = sequences.iter().copied().min().unwrap_or(0);
        let prev_sequence = if prev_sequence > 0 {
            prev_sequence - 1
        } else {
            0
        };
        Self {
            folder: upd.folder.clone(),
            files: upd.files.iter().map(|f| f.clone().into()).collect(),
            last_sequence: last_sequence.min(i64::MAX as u64) as i64,
            prev_sequence: prev_sequence.min(i64::MAX as u64) as i64,
        }
    }
}

impl From<IndexUpdate> for syncthing_core::types::IndexUpdate {
    fn from(upd: IndexUpdate) -> Self {
        Self {
            folder: upd.folder,
            files: upd.files.into_iter().map(Into::into).collect(),
        }
    }
}
