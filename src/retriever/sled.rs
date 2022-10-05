use std::path::Path;

pub struct SledRetriever {
    _db: sled::Db,
    paths: sled::Tree,
    files: sled::Tree,
}

#[derive(Debug)]
pub enum Error {
    Init(sled::Error),
    GetFileIndexByPath(sled::Error),
    PathNotFound,
    GetFileByIndex(sled::Error),
    FileIndexNotFound,
}
type Result<T, E = Error> = std::result::Result<T, E>;

impl SledRetriever {
    pub fn init<P: AsRef<Path>>(path: P) -> Result<Self> {
        let config = sled::Config::new()
            .path(path)
            .cache_capacity(128 * 1024 * 1024)
            .use_compression(true);
        //.compression_factor(22);
        let db = config.open().map_err(Error::Init)?;
        let paths = db.open_tree("paths").map_err(Error::Init)?;
        let files = db.open_tree("files").map_err(Error::Init)?;
        Ok(Self {
            _db: db,
            paths,
            files,
        })
    }
}

impl super::Retriever for SledRetriever {
    type Error = Error;

    fn file_by_path(&self, path: &str) -> Result<Vec<u8>, Self::Error> {
        let index = self
            .paths
            .get(path)
            .map_err(Error::GetFileIndexByPath)?
            .ok_or(Error::PathNotFound)?;
        let data = self
            .files
            .get(index)
            .map_err(Error::GetFileByIndex)?
            .ok_or(Error::FileIndexNotFound)?;
        Ok(data.as_ref().to_owned())
    }
}

impl Into<crate::GetImageError> for Error {
    fn into(self) -> crate::GetImageError {
        crate::GetImageError::SledRetrieve(self)
    }
}
