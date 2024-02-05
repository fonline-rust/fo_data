use std::path::PathBuf;

use fo_data::{FileInfo, FoRetriever, RetrieverExt};
use fo_proto_format::ProtoItem;

#[derive(Debug, Default)]
pub struct UnusedArt {
    pub size: u64,
    pub files: Vec<FileInfo>,
    pub errors: Vec<UnusedArtError>,
}

#[derive(Debug)]
pub struct UnusedArtError {
    pub conventional_path: String,
    pub file_location: Option<PathBuf>,
    pub error: String,
}

impl UnusedArt {
    pub fn prepare<'a>(protos: impl Iterator<Item = &'a ProtoItem>) -> UnusedArtFinder {
        let mut conventional_path = String::new();
        let mut hash = |path: Option<&str>| {
            if let Some(path) = path {
                fformat_utils::write_conventional_path(path, &mut conventional_path);
                Some(fo_data::hash(conventional_path.as_bytes()))
            } else {
                None
            }
        };
        UnusedArtFinder {
            hashes: protos.flat_map(|item| [
                hash(Some(&item.pic_map)),
                hash(item.pic_inv.as_deref()),
            ]).flatten().collect()
        }
    }    
}

pub struct UnusedArtFinder {
    hashes: Vec<u32>,
}

impl UnusedArtFinder {
    pub fn find(&self, retriever: &FoRetriever) -> UnusedArt {
        let mut res = UnusedArt::default();
        let mut files = retriever.registry().files().clone();
        for &hash in &self.hashes {
            if let Some(file_info) = files.forget_file_by_hash(hash) {
                match retriever.get_deps(file_info.conventional_path()) {
                    Ok(deps) => {
                        for dep in deps {
                            files.forget_file(&dep);
                        }
                    }
                    Err(err) => {
                        let file_location = file_info.location(retriever.registry());
                        res.errors.push(UnusedArtError {
                            conventional_path: file_info.conventional_path().to_owned(),
                            file_location: file_location.cloned(),
                            error: format!("Can't get deps: {err:?}"),
                        })
                    }
                }
            }
        }
        for file in files.drain_files() {
            if let Some(art) = file.conventional_path().strip_prefix("art/") {
                if let Some((
                    "critters" | "geometry" | "intrface" | "skilldex" | "splash" | "tracers" | "tiles",
                    _,
                )) = art.split_once('/')
                {
                    continue;
                }
                res.size += file.size();
                res.files.push(file)
            }
        }
        res
    }
}
