//mod converter;
mod converter;
pub mod crawler;
pub mod datafiles;
pub mod fofrm;
pub mod frm;
pub mod palette;
mod registry_cache;
pub mod retriever;

use std::{
    collections::{hash_map::DefaultHasher, BTreeMap},
    hash::Hasher,
    path::{Path, PathBuf},
    sync::Arc,
};

use nom_prelude::nom;
use serde::{Deserialize, Serialize};
pub type PathMap<K, V> = BTreeMap<K, V>;
pub type ChangeTime = std::time::SystemTime;
#[cfg(feature = "sled-retriever")]
pub use retriever::sled::SledRetriever;

pub use crate::{
    converter::{Converter, GetImageError, RawImage},
    palette::Palette,
    retriever::{fo::FoRetriever, Retriever},
};
use crate::{crawler::Files, registry_cache::FoRegistryCache};

pub type NomVerboseSliceError<'a> = nom::Err<nom::error::VerboseError<&'a [u8]>>;
pub type NomSliceErrorKind<'a> = nom::Err<(&'a [u8], nom::error::ErrorKind)>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum FileLocation {
    Archive {
        index: u16,
        original_path: String,
        compressed_size: u64,
    },
    Local {
        original_path: PathBuf,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileInfo {
    location: FileLocation,
    conventional_path: String,
}

impl FileInfo {
    pub fn new_in_archive(
        conventional_path: String,
        archive_index: u16,
        original_path: String,
        compressed_size: u64,
    ) -> Self {
        FileInfo {
            location: FileLocation::Archive {
                index: archive_index,
                original_path,
                compressed_size,
            },
            conventional_path,
        }
    }

    pub fn new_local(conventional_path: String, original_path: PathBuf) -> Self {
        FileInfo {
            location: FileLocation::Local { original_path },
            conventional_path,
        }
    }

    pub fn location<'a>(&'a self, data: &'a FoRegistry) -> Option<&'a std::path::PathBuf> {
        match &self.location {
            &FileLocation::Archive { index, .. } => data
                .archives
                .get(index as usize)
                .map(|archive| &archive.path),
            FileLocation::Local { original_path } => Some(original_path),
        }
    }

    pub fn hash(&self) -> u32 {
        hash(self.conventional_path.as_bytes())
    }

    pub fn conventional_path(&self) -> &str {
        &self.conventional_path
    }
}

pub fn conventional_hash(path: &str) -> u32 {
    let conventional_path = fformat_utils::make_path_conventional(path);
    hash(conventional_path.as_bytes())
}

pub fn hash(bytes: &[u8]) -> u32 {
    crc32fast::hash(bytes)
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct FoArchive {
    changed: ChangeTime,
    path: std::path::PathBuf,
}

pub struct FileData {
    pub data_type: DataType,
    pub data: bytes::Bytes,
    pub dimensions: (u32, u32),
    pub offset: (i16, i16),
}

#[derive(Debug, PartialEq)]
pub enum FileType {
    Png,
    Frm,
    Gif,
    FoFrm,
    Unsupported(String),
    Unknown,
}

#[derive(Debug, Hash)]
pub enum DataType {
    Png,
    Rgba,
}

#[derive(Debug)]
pub enum DataInitError {
    LoadPalette(palette::Error),
    Datafiles(datafiles::Error),
    GatherPaths(Box<crawler::Error>),
    CacheSerialize(bincode::Error),
    CacheDeserialize(bincode::Error),
    CacheIO(std::io::Error),
    DataFolderMissing,
    CacheStale,
    CacheIncompatible,
}

pub struct FoData<R = FoRetriever> {
    pub retriever: R,
    pub palette: Palette,
}
impl FoData {
    pub fn init<P: AsRef<Path>, P2: AsRef<Path>>(
        client_root: P,
        palette_path: P2,
    ) -> Result<Self, DataInitError> {
        let registry = FoRegistry::init(client_root)?;
        let retriever = registry.into_retriever();

        let palette = palette::load_palette(palette_path).map_err(DataInitError::LoadPalette)?;
        let palette = palette.colors_multiply(4);

        Ok(Self { retriever, palette })
    }
}
impl<R> FoData<R> {
    pub fn converter(&self) -> Converter<'_, '_, R> {
        Converter::new(&self.retriever, &self.palette)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheMetadata {
    changed: ChangeTime,
    local_paths_len: u32,
    local_paths_hash: u64,
}

impl CacheMetadata {
    fn new(local_paths: impl ExactSizeIterator<Item = u32>) -> Self {
        let local_paths_len = local_paths.len() as u32;

        let mut hasher = DefaultHasher::new();
        for hash in local_paths {
            hasher.write_u32(hash);
        }
        let local_paths_hash = hasher.finish();
        Self {
            local_paths_len,
            local_paths_hash,
            ..Default::default()
        }
    }
}

impl Default for CacheMetadata {
    fn default() -> Self {
        Self {
            changed: ChangeTime::now(),
            local_paths_len: 0,
            local_paths_hash: 0,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FoRegistry {
    cache_metadata: CacheMetadata,
    archives: Vec<FoArchive>,
    files: Files,
    dirs: Dirs,
}

pub type FoRegistryArc = Arc<FoRegistry>;

const CACHE_PATH: &str = "fo_data.bin";
const DATA_PATH: &str = "data";

impl FoRegistry {
    fn version() -> u32 {
        1
    }

    pub fn stub() -> Self {
        FoRegistry::default()
    }

    fn recover_from_cache<P: AsRef<Path>>(
        client_root: P,
        new_cache_metadata: &CacheMetadata,
    ) -> Result<Self, DataInitError> {
        type Error = DataInitError;

        let cache_file = std::fs::File::open(CACHE_PATH).map_err(Error::CacheIO)?;
        let cache_changed = cache_file
            .metadata()
            .map_err(Error::CacheIO)?
            .modified()
            .map_err(Error::CacheIO)?;
        let reader = std::io::BufReader::new(cache_file);
        let cache: FoRegistryCache<_> =
            bincode::deserialize_from(reader).map_err(Error::CacheDeserialize)?;
        let data: FoRegistry = cache.into_data()?;

        let datafiles_changetime =
            datafiles::datafiles_changetime(client_root).map_err(Error::Datafiles)?;

        let cache_changed = cache_changed.min(data.cache_metadata.changed);
        if datafiles_changetime > cache_changed {
            return Err(Error::CacheStale);
        }

        if new_cache_metadata.local_paths_len != data.cache_metadata.local_paths_len
            || new_cache_metadata.local_paths_hash != data.cache_metadata.local_paths_hash
        {
            return Err(Error::CacheStale);
        }

        // TODO: gather new change times instead
        for archive in &data.archives {
            if archive.changed > cache_changed {
                return Err(Error::CacheStale);
            }
        }

        Ok(data)
    }

    pub fn init(client_root: impl AsRef<Path>) -> Result<Self, DataInitError> {
        type Error = DataInitError;

        let local_paths = crawler::gather_local_paths(
            client_root
                .as_ref()
                .join(DATA_PATH)
                .canonicalize()
                .map_err(|_| Error::DataFolderMissing)?,
        );

        let cache_metadata = CacheMetadata::new(local_paths.iter().map(|(hash, _)| *hash));

        match Self::recover_from_cache(&client_root, &cache_metadata) {
            Err(err) => println!("FoData recovery failed: {:?}", err),
            ok => return ok,
        }

        let archives = datafiles::parse_datafile(client_root).map_err(Error::Datafiles)?;

        let paths_in_archives = crawler::gather_paths_in_archives(&archives);

        let mut files = Files::default();
        files
            .reconcile_paths(paths_in_archives.into_iter().flatten(), |_, _, _| Ok(()))
            .map_err(Error::GatherPaths)?;

        files
            .reconcile_paths(local_paths.into_iter(), |file, old, new| match &old {
                FileLocation::Local { .. } => Err(crawler::Error::LocalRewrite {
                    file,
                    old: old.clone(),
                    new: new.clone(),
                }),
                _ => Ok(()),
            })
            .map_err(Error::GatherPaths)?;

        let mut dirs = Dirs::default();
        for path in files.paths() {
            dirs.register(path, FoMetadata::File);
        }

        let fo_data = FoRegistry {
            cache_metadata,
            archives,
            files,
            dirs,
            //palette,
        };
        {
            let cache_file = std::fs::File::create(CACHE_PATH).map_err(Error::CacheIO)?;
            let mut writer = std::io::BufWriter::new(cache_file);
            let cache = FoRegistryCache::new(&fo_data);
            bincode::serialize_into(&mut writer, &cache).map_err(Error::CacheSerialize)?;
        }
        Ok(fo_data)
    }

    pub fn count_archives(&self) -> usize {
        self.archives.len()
    }

    pub fn into_retriever(self) -> FoRetriever {
        FoRetriever::new(Arc::new(self))
    }

    pub fn files(&self) -> &Files {
        &self.files
    }

    fn is_dir(&self, path: &str) -> bool {
        path.is_empty() || self.dirs.map.get(path.trim_end_matches('/')).is_some()
    }

    pub fn metadata(&self, path: &str) -> Option<FoMetadata> {
        if let Some(_file_info) = self.files.file_info(path) {
            Some(FoMetadata::File)
        } else if self.is_dir(path) {
            Some(FoMetadata::Dir)
        } else {
            None
        }
    }

    pub fn file_location(&self, path: &str) -> Option<&Path> {
        self.files
            .file_info(path)?
            .location(self)
            .map(AsRef::as_ref)
    }

    pub fn ls_dir<'a>(&'a self, path: &'a str) -> Option<impl 'a + Iterator<Item = &'a str>> {
        Some(
            self.dirs
                .map
                .get(path.trim_end_matches('/'))?
                .iter()
                .map(|(entry, _)| entry.as_str()),
        )
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Dirs {
    map: PathMap<String, PathMap<String, FoMetadata>>,
}

impl Dirs {
    fn parent(path: &str) -> &str {
        path.trim_end_matches('/')
            .rsplit_once('/')
            .unwrap_or(("", &path))
            .0
    }

    fn register(&mut self, path: &str, metadata: FoMetadata) {
        let parent = Self::parent(path);

        let entries = self.map.entry(parent.to_owned()).or_default();
        if entries.contains_key(path) {
            return;
        }
        entries.insert(path.to_owned(), metadata);

        if !parent.is_empty() {
            self.register(parent, FoMetadata::Dir);
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum FoMetadata {
    File,
    Dir,
}

trait PathError<T, E>: Sized {
    fn path_err<E2>(self, path: &Path, fun: fn(PathBuf, E) -> E2) -> Result<T, E2>;
    fn paths_err<E2>(
        self,
        path1: &Path,
        path2: &Path,
        fun: fn(PathBuf, PathBuf, E) -> E2,
    ) -> Result<T, E2>;
    fn just_path<E2>(self, path: &Path, fun: fn(PathBuf) -> E2) -> Result<T, E2>;
}
impl<T, E> PathError<T, E> for Result<T, E> {
    fn path_err<E2>(self, path: &Path, fun: fn(PathBuf, E) -> E2) -> Result<T, E2> {
        match self {
            Ok(ok) => Ok(ok),
            Err(err) => Err(fun(path.into(), err)),
        }
    }

    fn paths_err<E2>(
        self,
        path1: &Path,
        path2: &Path,
        fun: fn(PathBuf, PathBuf, E) -> E2,
    ) -> Result<T, E2> {
        match self {
            Ok(ok) => Ok(ok),
            Err(err) => Err(fun(path1.into(), path2.into(), err)),
        }
    }

    fn just_path<E2>(self, path: &Path, fun: fn(PathBuf) -> E2) -> Result<T, E2> {
        match self {
            Ok(ok) => Ok(ok),
            Err(_err) => Err(fun(path.into())),
        }
    }
}

#[cfg(test)]
mod test_stuff {
    use std::path::{Path, PathBuf};

    pub const CLIENT_FOLDER: &str = "../../../CL4RP";
    pub const TEST_ASSETS_FOLDER: &str = "../../../test_assets";
    pub fn test_assets() -> PathBuf {
        Path::new(TEST_ASSETS_FOLDER).to_owned()
    }
    pub fn palette_path() -> PathBuf {
        Path::new(TEST_ASSETS_FOLDER).join("COLOR.PAL")
    }

    #[cfg(not(feature = "sled-retriever"))]
    pub fn test_data() -> crate::FoData {
        crate::FoData::init(CLIENT_FOLDER, palette_path()).unwrap()
    }

    #[cfg(not(feature = "sled-retriever"))]
    pub fn test_retriever() -> crate::FoRetriever {
        crate::FoRegistry::init(CLIENT_FOLDER)
            .unwrap()
            .into_retriever()
    }

    #[cfg(feature = "sled-retriever")]
    pub fn test_retriever() -> &'static crate::SledRetriever {
        static RETRIEVER: once_cell::sync::Lazy<SledRetriever> = once_cell::sync::Lazy::new(|| {
            crate::SledRetriever::init(test_assets().join("db/assets"), palette_path()).unwrap()
        });
        &*RETRIEVER
    }
}
#[cfg(test)]
use test_stuff::*;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn load_frm_from_zip_and_convert_to_png() {
        let data = test_data();
        //"art/tiles/FOM1000.FRM"
        let image = data.converter().get_png("art/tiles/fom1000.frm").unwrap();
        std::fs::write(test_assets().join("output/FOM1000.png"), image.data).unwrap();
    }

    fn save_frame<'a>(frame: &'a frm::Frame<'a>, palette: &[(u8, u8, u8)], path: impl AsRef<Path>) {
        let image = image::GrayImage::from_raw(
            frame.width as u32,
            frame.height as u32,
            frame.data.to_owned(),
        )
        .unwrap();
        let colored = image.expand_palette(palette, None);
        colored.save(path).unwrap();
    }

    #[test]
    fn colored_tile() {
        let file = std::fs::read(palette_path()).unwrap();
        let (_, palette) = palette::palette_verbose(&file).unwrap();

        let file = std::fs::read(test_assets().join("EDG1001.FRM")).unwrap();
        let (_, frm) = frm::frm_verbose(&file).unwrap();

        save_frame(
            &frm.directions[0].frames[0],
            palette.colors_multiply(1).colors_tuples(),
            test_assets().join("output/EDG1001_1.png"),
        );
        save_frame(
            &frm.directions[0].frames[0],
            palette.colors_multiply(2).colors_tuples(),
            test_assets().join("output/EDG1001_2.png"),
        );
        save_frame(
            &frm.directions[0].frames[0],
            palette.colors_multiply(3).colors_tuples(),
            test_assets().join("output/EDG1001_3.png"),
        );
        save_frame(
            &frm.directions[0].frames[0],
            palette.colors_multiply(4).colors_tuples(),
            test_assets().join("output/EDG1001_4.png"),
        );
    }

    #[test]
    fn colored_animation() {
        let file = std::fs::read(palette_path()).unwrap();
        let (_, palette) = palette::palette_verbose(&file).unwrap();
        let palette4 = palette.colors_multiply(4);

        let file = std::fs::read(test_assets().join("HMWARRAA.FRM")).unwrap();
        let (_, frm) = frm::frm_verbose(&file).unwrap();

        for (dir_index, dir) in frm.directions.iter().enumerate() {
            for (frame_index, frame) in dir.frames.iter().enumerate() {
                save_frame(
                    frame,
                    palette4.colors_tuples(),
                    format!(
                        "{}/output/HMWARRAA_{}_{}.png",
                        TEST_ASSETS_FOLDER, dir_index, frame_index
                    ),
                );
            }
        }
    }

    #[test]
    fn print_frm_animation_info() {
        let retriever = test_retriever();
        let bytes = retriever.file_by_path("art/scenery/gizsign.frm").unwrap();
        let (_rest, frm) = frm::frm_verbose(&bytes).unwrap();
        println!("{:?}", frm);
    }
}
