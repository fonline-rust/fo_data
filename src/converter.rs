use std::io::Cursor;

use self::frm::FrmParseError;
use crate::*;

#[derive(Debug)]
pub enum GetImageError {
    FileType(FileType),
    Utf8(std::str::Utf8Error),
    FrmParse(FrmParseError),
    FoFrmParse(fofrm::FoFrmError),
    NoParentFolder,
    InvalidRelativePath(String, String),
    NoDirection,
    NoFrame,
    ImageFromRaw,
    ImageWrite(image::ImageError),
    PngDecode(image::ImageError),
    Recursion(usize, Box<GetImageError>),
    RecursionLimit,
    NoPallete,
    FoRetrieve(<FoRetriever as Retriever>::Error),
    #[cfg(feature = "sled-retriever")]
    SledRetrieve(<crate::retriever::sled::SledRetriever as Retriever>::Error),
}
impl GetImageError {
    fn recursion(self) -> Self {
        use GetImageError::*;
        match self {
            Recursion(num, origin) => Recursion(num + 1, origin),
            origin => Recursion(0, Box::new(origin)),
        }
    }
}
pub struct Converter<'r, 'p, R> {
    retriever: &'r R,
    palette: &'p Palette,
}
impl<'r, 'p, R> Converter<'r, 'p, R> {
    pub fn new(retriever: &'r R, palette: &'p Palette) -> Self {
        Self { retriever, palette }
    }
}

impl<'r, 'p, R: Retriever> Converter<'r, 'p, R>
where
    R::Error: Into<GetImageError>,
{
    pub fn get_png(&self, path: &str) -> Result<FileData, GetImageError> {
        let raw = get_raw(self.retriever, path, 0, Some(self.palette.colors_tuples()))?;
        raw.into_png().map_err(GetImageError::ImageWrite)
    }

    pub fn get_rgba(&self, path: &str) -> Result<RawImage, GetImageError> {
        get_raw(self.retriever, path, 0, Some(self.palette.colors_tuples()))
    }
}

#[derive(Debug, Clone)]
pub struct RawImage {
    pub image: image::RgbaImage,
    pub offset_x: i16,
    pub offset_y: i16,
}

impl RawImage {
    fn into_png(self) -> Result<FileData, image::ImageError> {
        let dimensions = self.image.dimensions();
        let size = (dimensions.0 as usize * dimensions.1 as usize * 4 + 512).next_power_of_two();
        let image = image::DynamicImage::ImageRgba8(self.image);
        let data = Vec::with_capacity(size);
        let mut cursor = Cursor::new(data);

        image.write_to(&mut cursor, image::ImageFormat::Png)?;
        Ok(FileData {
            data: cursor.into_inner().into(),
            data_type: DataType::Png,
            dimensions,
            offset: (self.offset_x, self.offset_y),
        })
    }
}

fn get_raw<R: Retriever>(
    retriever: &R,
    path: &str,
    recursion: usize,
    palette: Option<&[(u8, u8, u8)]>,
) -> Result<RawImage, GetImageError>
where
    R::Error: Into<GetImageError>,
{
    const RECURSION_LIMIT: usize = 1;
    if recursion > RECURSION_LIMIT {
        return Err(GetImageError::RecursionLimit);
    }
    let file_type = retriever::recognize_type(path);

    Ok(match file_type {
        FileType::Png => {
            let data = retriever.file_by_path(path).map_err(Into::into)?;
            let slice = &data[..];

            let dynamic = image::load_from_memory_with_format(slice, image::ImageFormat::Png)
                .map_err(GetImageError::PngDecode)?;
            let mut image = dynamic.into_rgba8();
            let (width, height) = image.dimensions();

            image.pixels_mut().for_each(|pixel| {
                if pixel.0 == [0, 0, 255, 255] {
                    pixel.0 = [0, 0, 0, 0];
                }
            });

            RawImage {
                image,
                offset_x: width as i16 / -2,
                offset_y: -(height as i16),
            }
        }
        FileType::Frm => {
            let palette = palette.ok_or(GetImageError::NoPallete)?;
            let data = retriever.file_by_path(path).map_err(Into::into)?;
            let frm = frm::frm(&data).map_err(GetImageError::FrmParse)?;
            let frame_number = 0;

            let direction = frm.directions.first().ok_or(GetImageError::NoDirection)?;
            let frame = direction
                .frames
                .get(frame_number)
                .ok_or(GetImageError::NoFrame)?;

            let offsets = direction.frames.iter().skip(1).take(frame_number);
            let offset_x: i16 = offsets.clone().map(|frame| frame.offset_x).sum();
            let offset_y: i16 = offsets.map(|frame| frame.offset_y).sum();

            let image = image::GrayImage::from_raw(
                frame.width as u32,
                frame.height as u32,
                frame.data.to_owned(),
            )
            .ok_or(GetImageError::ImageFromRaw)?;
            let image = image.expand_palette(palette, Some(0));
            RawImage {
                image,
                offset_x: direction.shift_x + offset_x - frame.width as i16 / 2,
                offset_y: direction.shift_y + offset_y - frame.height as i16,
            }
        }
        FileType::FoFrm => {
            let parent_folder = std::path::Path::new(path)
                .parent()
                .ok_or(GetImageError::NoParentFolder)?
                .to_owned();
            let data = retriever.file_by_path(path).map_err(Into::into)?;

            let string = std::str::from_utf8(&data).map_err(GetImageError::Utf8)?;
            let fofrm = fofrm::parse_verbose(string).map_err(GetImageError::FoFrmParse)?;
            let frame_number = 0;

            let direction = fofrm.directions.first().ok_or(GetImageError::NoDirection)?;
            let frame = direction
                .frames
                .get(frame_number)
                .ok_or(GetImageError::NoFrame)?;

            let offsets = direction.frames.iter().skip(1).take(frame_number);
            let mut offset_x: i16 = offsets.clone().map(|frame| frame.next_x.unwrap_or(0)).sum();
            let mut offset_y: i16 = offsets.map(|frame| frame.next_y.unwrap_or(0)).sum();

            offset_x += direction.offset_x.or(fofrm.offset_x).unwrap_or(0);
            offset_y += direction.offset_y.or(fofrm.offset_y).unwrap_or(0);

            let relative_path = frame.frm.ok_or(GetImageError::NoFrame)?;
            //dbg!(&full_path, &relative_path);

            //dbg!(&full_path);

            let full_path = resolve_dep_path(&parent_folder, relative_path).ok_or_else(|| {
                GetImageError::InvalidRelativePath(path.into(), relative_path.into())
            })?;

            let mut image = get_raw(retriever, &full_path, recursion + 1, palette)
                .map_err(GetImageError::recursion)?;
            image.offset_x += offset_x;
            image.offset_y += offset_y;
            image
        }
        _ => return Err(GetImageError::FileType(file_type)),
    })
}

pub trait RetrieverExt: Retriever {
    fn get_deps(&self, path: &str) -> Result<Vec<String>, GetImageError>
    where
        Self::Error: Into<GetImageError>,
    {
        let file_type = retriever::recognize_type(path);
        match file_type {
            FileType::FoFrm => {
                let parent_folder = std::path::Path::new(path)
                    .parent()
                    .ok_or(GetImageError::NoParentFolder)?
                    .to_owned();
                let data = self.file_by_path(path).map_err(Into::into)?;

                let string = std::str::from_utf8(&data).map_err(GetImageError::Utf8)?;
                let fofrm = fofrm::parse_verbose(string).map_err(GetImageError::FoFrmParse)?;

                fofrm
                    .directions
                    .iter()
                    .flat_map(|direction| direction.frames.iter())
                    .filter_map(|frame| frame.frm)
                    .map(|relative_path| {
                        resolve_dep_path(&parent_folder, relative_path).ok_or_else(|| {
                            GetImageError::InvalidRelativePath(path.into(), relative_path.into())
                        })
                    })
                    .collect()
            }
            _ => Ok(vec![]),
        }
    }
}

impl<T: Retriever> RetrieverExt for T {}

fn resolve_dep_path(parent_folder: &Path, relative_path: &str) -> Option<String> {
    let mut full_path = parent_folder.to_owned();
    for component in std::path::Path::new(relative_path).components() {
        use std::path::Component;
        if !match component {
            Component::ParentDir => full_path.pop(),
            Component::Normal(str) => {
                full_path.push(str);
                true
            }
            _ => false,
        } {
            return None;
        }
    }
    Some(fformat_utils::make_path_conventional(
        full_path
            .to_str()
            .expect("Convert full path back to string"),
    ))
}
