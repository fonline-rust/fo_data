use std::path::PathBuf;

use parking_lot::{MappedMutexGuard as Guard, Mutex, MutexGuard};
use thiserror::Error;

use crate::{FileLocation, FoRegistryArc, PathError};

#[derive(Debug, Error)]
pub enum Error {
    #[error("path not found")]
    NotFound,
    #[error("invalid archive index")]
    InvalidArchiveIndex,
    #[error("can't open archve: {0}")]
    OpenArchive(PathBuf, std::io::Error),
    #[error("zip err: {0}")]
    Zip(zip::result::ZipError),
    #[error("unsupporte file location")]
    UnsupportedFileLocation,
    #[error("archive io error: {0}")]
    ArchiveRead(std::io::Error),
    #[error("local io error: {0}")]
    LocalIO(std::io::Error),
}

type Archive = zip::ZipArchive<std::io::BufReader<std::fs::File>>;

pub struct FoRetriever {
    archives: Vec<Mutex<Option<Box<Archive>>>>,
    data: FoRegistryArc,
}

impl FoRetriever {
    pub fn new(data: FoRegistryArc) -> Self {
        let mut archives = Vec::new();
        archives.resize_with(data.archives.len(), Default::default);
        Self { archives, data }
    }

    fn get_archive(&self, archive_index: usize) -> Result<Guard<Archive>, Error> {
        use std::io::BufReader;

        let mut guard = self.archives[archive_index].lock();

        if guard.is_none() {
            let archive = self
                .data
                .archives
                .get(archive_index)
                .ok_or(Error::InvalidArchiveIndex)?;
            let archive_file =
                std::fs::File::open(&archive.path).path_err(&archive.path, Error::OpenArchive)?;
            let archive_buf_reader = BufReader::with_capacity(1024, archive_file);
            let archive = zip::ZipArchive::new(archive_buf_reader).map_err(Error::Zip)?;
            *guard = Some(Box::new(archive));
        }
        Ok(MutexGuard::map(guard, |option| {
            &mut **option.as_mut().expect("Should be some")
        }))
    }

    pub fn registry(&self) -> &FoRegistryArc {
        &self.data
    }

    pub fn file_by_info(&self, file_info: &crate::FileInfo) -> Result<Vec<u8>, Error> {
        use std::io::Read;

        match file_info.location {
            FileLocation::Archive {
                index: archive_index,
                ref original_path,
                ..
            } => {
                let mut archive = self.get_archive(archive_index as usize)?;

                let mut file = archive.by_name(original_path).map_err(Error::Zip)?;
                let mut buffer = Vec::with_capacity(file.size() as usize);
                file.read_to_end(&mut buffer).map_err(Error::ArchiveRead)?;
                Ok(buffer)
            }
            FileLocation::Local { ref original_path } => {
                std::fs::read(original_path).map_err(Error::LocalIO)
            }
        }
    }
}

impl super::Retriever for FoRetriever {
    type Error = Error;

    fn file_by_path(&self, path: &str) -> Result<Vec<u8>, Self::Error> {
        let file_info = self.data.files.file_info(path).ok_or(Error::NotFound)?;

        self.file_by_info(file_info)
    }
}

impl From<Error> for crate::GetImageError {
    fn from(val: Error) -> Self {
        crate::GetImageError::FoRetrieve(val)
    }
}
