//mod converter;
mod converter;
pub mod crawler;
pub mod datafiles;
pub mod fofrm;
pub mod frm;
pub mod palette;
pub mod retriever;

use std::{collections::BTreeMap, path::{Path, PathBuf}, sync::Arc};

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

#[derive(Debug, Serialize, Deserialize)]
pub enum FileLocation {
    Archive(u16),
    Local,
}
impl Default for FileLocation {
    fn default() -> Self {
        FileLocation::Local
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FileInfo {
    location: FileLocation,
    original_path: String,
    compressed_size: u64,
}
impl FileInfo {
    pub fn location<'a>(&self, data: &'a FoRegistry) -> Option<&'a std::path::PathBuf> {
        match self.location {
            FileLocation::Archive(index) => data
                .archives
                .get(index as usize)
                .map(|archive| &archive.path),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
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
    GatherPaths(crawler::Error),
    CacheSerialize(bincode::Error),
    CacheDeserialize(bincode::Error),
    CacheIO(std::io::Error),
    CacheStale,
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
pub struct FoRegistry {
    changed: ChangeTime,
    archives: Vec<FoArchive>,
    files: PathMap<String, FileInfo>,
    dirs: Dirs,
    //cache: HashMap<(String, OutputType), FileData>,
    //palette: Palette,
}

const CACHE_PATH: &str = "fo_data.bin";
impl FoRegistry {
    pub fn stub() -> Self {
        FoRegistry {
            changed: ChangeTime::now(),
            archives: Default::default(),
            files: Default::default(),
            dirs: Default::default(),
            //palette: Default::default(),
        }
    }

    fn recover_from_cache<P: AsRef<Path>>(client_root: P) -> Result<Self, DataInitError> {
        type Error = DataInitError;
        let cache_file = std::fs::File::open(CACHE_PATH).map_err(Error::CacheIO)?;
        let cache_changed = cache_file
            .metadata()
            .map_err(Error::CacheIO)?
            .modified()
            .map_err(Error::CacheIO)?;
        let reader = std::io::BufReader::new(cache_file);
        let fo_data: FoRegistry =
            bincode::deserialize_from(reader).map_err(Error::CacheDeserialize)?;
        let datafiles_changetime =
            datafiles::datafiles_changetime(client_root).map_err(Error::Datafiles)?;
        let cache_changed = cache_changed.min(fo_data.changed);
        if datafiles_changetime > cache_changed {
            return Err(Error::CacheStale);
        }
        for archive in &fo_data.archives {
            if archive.changed > cache_changed {
                return Err(Error::CacheStale);
            }
        }
        Ok(fo_data)
    }
    /*
    fn cut_paths<V>(map: &PathMap<String, V>) -> PathMap<String, ()> {
        map.keys().filter_map(|path| Some((path.rsplit_once('/')?.0.to_owned(), ()))).collect()
    }

    
    fn compute_dirs<V>(files: &PathMap<String, V>) -> PathMap<String, DirEntry> {
        let dir = Self::cut_paths(&files);
        if dir.is_empty() {
            return Default::default();
        }

        let mut dirs_vec = vec![dir];
        loop {
            let dir = Self::cut_paths(dirs_vec.last().unwrap());
            if dir.is_empty() {
                break;
            }
            dirs_vec.push(dir);
        }
        let mut result = dirs_vec.swap_remove(0);
        for mut dir in dirs_vec {
            result.append(&mut dir);
        }
        result
    }
    */

    pub fn init(client_root: impl AsRef<Path>) -> Result<Self, DataInitError> {
        type Error = DataInitError;
        match Self::recover_from_cache(&client_root) {
            Err(err) => println!("FoData recovery failed: {:?}", err),
            ok => return ok,
        }

        let archives = datafiles::parse_datafile(client_root).map_err(Error::Datafiles)?;
        let files = crawler::gather_paths(&archives).map_err(Error::GatherPaths)?;
        let mut dirs = Dirs::default();
        for (path, _) in &files {
            dirs.register(path, FoMetadata::File);
        }

        let changed = ChangeTime::now();
        let fo_data = FoRegistry {
            changed,
            archives,
            files,
            dirs,
            //palette,
        };
        {
            let cache_file = std::fs::File::create(CACHE_PATH).map_err(Error::CacheIO)?;
            let mut writer = std::io::BufWriter::new(cache_file);
            bincode::serialize_into(&mut writer, &fo_data).map_err(Error::CacheSerialize)?;
        }
        Ok(fo_data)
    }

    pub fn count_archives(&self) -> usize {
        self.archives.len()
    }

    pub fn count_files(&self) -> usize {
        self.files.len()
    }

    pub fn into_retriever(self) -> FoRetriever {
        FoRetriever::new(Arc::new(self))
    }

    pub fn files(&self) -> impl ExactSizeIterator<Item = (&str, &FileInfo)> {
        self.files.iter().map(|(path, info)| (path.as_str(), info))
    }

    pub fn file_info(&self, path: &str) -> Option<&FileInfo> {
        self.files.get(path)
    }

    fn is_dir(&self, path: &str) -> bool {
        path.is_empty() || self.dirs.map.get(path.trim_end_matches('/')).is_some()
    }

    pub fn metadata(&self, path: &str) -> Option<FoMetadata> {
        if let Some(_file_info) = self.file_info(path) {
            Some(FoMetadata::File)
        } else if self.is_dir(path) {
            Some(FoMetadata::Dir)
        } else {
            None
        }
    }

    pub fn file_location(&self, path: &str) -> Option<&Path> {
        self.file_info(path)?.location(self).map(AsRef::as_ref)
    }
    /*
    fn walk_path<'a, V>(map: &'a PathMap<String, V>, path: &'a str) -> impl 'a + Iterator<Item = &'a str> {
        map.range::<str, _>((Bound::Excluded(path), Bound::Unbounded)).map_while(move |(key, _value)| {
            Some((key.as_str().strip_prefix(path)?, key.as_str()))
        }).filter_map(|(rel, absolute)| {
            if rel.trim_start_matches('/').contains('/') {
                println!("rel: {rel}, abs: {absolute}, None");
                None
            } else {
                println!("rel: {rel}, abs: {absolute}, Good");
                Some(absolute)
            }
        })
    }
    */

    pub fn ls_dir<'a>(&'a self, path: &'a str) -> Option<impl 'a + Iterator<Item = &'a str>> {
        Some(self.dirs.map.get(path.trim_end_matches('/'))?.iter().map(|(entry, _)| entry.as_str()))
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Dirs {
    map: PathMap<String, PathMap<String, FoMetadata>>,
}

impl Dirs {
    fn parent(path: &str) -> &str {
        path.trim_end_matches('/').rsplit_once('/').unwrap_or(("", &path)).0
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
    fn paths_err<E2>(self, path1: &Path, path2: &Path, fun: fn(PathBuf, PathBuf, E) -> E2) -> Result<T, E2>;
    fn just_path<E2>(self, path: &Path, fun: fn(PathBuf) -> E2) -> Result<T, E2>;
}
impl<T, E> PathError<T, E> for Result<T, E> {
    fn path_err<E2>(self, path: &Path, fun: fn(PathBuf, E) -> E2) -> Result<T, E2> {
        match self {
            Ok(ok) => Ok(ok),
            Err(err) => Err(fun(path.into(), err)),
        }
    }
    fn paths_err<E2>(self, path1: &Path, path2: &Path, fun: fn(PathBuf, PathBuf, E) -> E2) -> Result<T, E2> {
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
                    &frame,
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
