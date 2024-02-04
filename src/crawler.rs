use std::{collections::hash_map::Entry, io::BufReader, path::Path};

use nohash_hasher::IntMap;
use rayon::prelude::IntoParallelRefIterator;
use serde::{Deserialize, Serialize};

use crate::{FileInfo, FileLocation};

#[derive(Debug)]
pub enum Error {
    Conflict {
        hash: u32,
        old: FileInfo,
        new: FileInfo,
    },
    LocalRewrite {
        file: String,
        old: FileLocation,
        new: FileLocation,
    },
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Files {
    inner: IntMap<u32, FileInfo>,
}

impl Files {
    pub fn get(&self, hash: u32) -> Option<&FileInfo> {
        self.inner.get(&hash)
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.inner
            .values()
            .map(|info| info.conventional_path.as_str())
    }

    pub fn infos(&self) -> impl ExactSizeIterator<Item = &FileInfo> {
        self.inner.values()
    }

    pub fn file_info(&self, path: &str) -> Option<&FileInfo> {
        self.inner.get(&crate::hash(path.as_bytes()))
    }

    pub fn count_files(&self) -> usize {
        self.inner.len()
    }

    pub fn reconcile_paths(
        &mut self,
        paths: impl Iterator<Item = (u32, FileInfo)>,
        mut on_shadow: impl FnMut(String, &FileLocation, &FileLocation) -> Result<(), Error>,
    ) -> Result<(), Box<Error>> {
        for (hash, file_info) in paths {
            match self.inner.entry(hash) {
                Entry::Vacant(entry) => {
                    entry.insert(file_info);
                }
                Entry::Occupied(mut entry) => {
                    let old = entry.get_mut();
                    if old.conventional_path != file_info.conventional_path {
                        return Err(Box::new(Error::Conflict {
                            hash,
                            old: entry.remove(),
                            new: file_info,
                        }));
                    }
                    on_shadow(
                        file_info.conventional_path,
                        &old.location,
                        &file_info.location,
                    )?;
                    old.location = file_info.location;
                }
            }
        }
        Ok(())
    }
}

#[deprecated]
pub fn gather_paths(archives: &[crate::FoArchive]) -> IntMap<u32, FileInfo> {
    let paths = gather_paths_in_archives(archives);
    let mut files = Files::default();
    files
        .reconcile_paths(paths.into_iter().flatten(), |_, _, _| Ok(()))
        .unwrap();
    files.inner
}

pub fn gather_paths_in_archives(archives: &[crate::FoArchive]) -> Vec<Vec<(u32, FileInfo)>> {
    assert!(archives.len() <= u16::max_value() as usize);

    use rayon::prelude::{IndexedParallelIterator, ParallelIterator};
    archives
        .par_iter()
        .enumerate()
        .map(|(archive_index, archive)| {
            println!("Crawling {:?}", archive.path);
            let archive_file = std::fs::File::open(&archive.path).unwrap();
            let buf_reader = BufReader::with_capacity(1024, archive_file);
            let mut archive_zip = zip::ZipArchive::new(buf_reader).unwrap();
            let mut vec = Vec::with_capacity(archive_zip.len());
            for i in 0..archive_zip.len() {
                let entry = archive_zip.by_index(i).unwrap();
                if entry.is_dir() {
                    continue;
                }
                let entry_name = entry.name();
                let conventional_path = fformat_utils::make_path_conventional(entry_name);

                let file_info = FileInfo::new_in_archive(
                    conventional_path,
                    archive_index as u16,
                    entry_name.to_owned(),
                    entry.compressed_size(),
                );

                let hash = file_info.hash();
                vec.push((hash, file_info));
            }
            vec
        })
        .collect()
}

pub fn gather_local_paths(parent: impl AsRef<Path>) -> Vec<(u32, FileInfo)> {
    walkdir::WalkDir::new(parent.as_ref())
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let stripped = e.path().strip_prefix(parent.as_ref()).ok()?;
            let string = stripped.to_str()?;
            let conventional_path = fformat_utils::make_path_conventional(string);

            let file_info = FileInfo::new_local(conventional_path, e.into_path());
            let hash = file_info.hash();
            Some((hash, file_info))
        })
        .collect()
}

pub struct ShadowedFile<'a> {
    pub name: String,
    pub size: u64,
    pub first_source: &'a Path,
    pub second_source: &'a Path,
}

pub fn shadowed_files(archives: &[crate::FoArchive]) -> Result<Vec<ShadowedFile>, Box<Error>> {
    assert!(archives.len() <= u16::max_value() as usize);

    let mut shadowed = Vec::with_capacity(512);

    let paths = gather_paths_in_archives(archives);

    let mut files = Files::default();

    files.reconcile_paths(paths.into_iter().flatten(), |name, old, new| {
        if let (
            &FileLocation::Archive {
                index: old_index, ..
            },
            &FileLocation::Archive {
                index,
                compressed_size,
                ..
            },
        ) = (old, new)
        {
            shadowed.push(ShadowedFile {
                name,
                size: compressed_size,
                first_source: archives[old_index as usize].path.as_path(),
                second_source: archives[index as usize].path.as_path(),
            });
        }
        Ok(())
    })?;
    Ok(shadowed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[allow(deprecated)]
    fn test_gather_paths() {
        let archives = crate::datafiles::parse_datafile(crate::CLIENT_FOLDER).unwrap();

        for (_hash, info) in gather_paths(&archives) {
            match info.location {
                FileLocation::Local { .. } => {
                    println!("{:?} => local", &info.conventional_path);
                }
                FileLocation::Archive { index, .. } => {
                    println!(
                        "{:?} => {:?}",
                        &info.conventional_path, &archives[index as usize]
                    );
                }
            }
        }
    }
}
