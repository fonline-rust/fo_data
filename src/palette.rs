use nom::number::complete::be_u8;
use nom_prelude::{
    count_cap, err_to_kind, nom, parse_struct, tuple, ErrorKind, IResult, ParseError,
};
use serde::{Deserialize, Serialize};

use crate::{NomSliceErrorKind, NomVerboseSliceError};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Palette {
    pub colors: Vec<Color>,
}

impl Palette {
    pub fn colors_tuples(&self) -> &[(u8, u8, u8)] {
        unsafe { std::mem::transmute(self.colors.as_slice()) }
    }

    pub fn map_colors(&self, map: impl Fn(u8) -> u8) -> Palette {
        let colors = self
            .colors
            .iter()
            .map(|color| Color {
                red: map(color.red),
                green: map(color.green),
                blue: map(color.blue),
            })
            .collect();
        Palette { colors }
    }

    pub fn colors_multiply(&self, val: u8) -> Palette {
        self.map_colors(|color| color.saturating_mul(val))
    }

    pub fn colors_multiply_f32(&self, val: f32) -> Palette {
        self.map_colors(|color| (color as f32 * val).round().clamp(0.0, 255.0) as u8)
    }
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Nom(ErrorKind),
}

pub fn load_palette<P: AsRef<std::path::Path>>(path: P) -> Result<Palette, Error> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(Error::Io)?;
    let mut palette_buf = [0u8; 256 * 3];
    file.read_exact(&mut palette_buf).map_err(Error::Io)?;
    let palette = err_to_kind(palette(&palette_buf)).map_err(Error::Nom)?;
    Ok(palette)
}

pub fn palette(buf: &[u8]) -> Result<(&[u8], Palette), NomSliceErrorKind> {
    let (rest, palette) = parse_palette(buf)?;
    Ok((rest, palette))
}

pub fn palette_verbose(buf: &[u8]) -> Result<(&[u8], Palette), NomVerboseSliceError> {
    let (rest, palette) = parse_palette(buf)?;
    Ok((rest, palette))
}

fn parse_palette<'a, Error: ParseError<&'a [u8]>>(
    i: &'a [u8],
) -> IResult<&'a [u8], Palette, Error> {
    let (i, colors) = count_cap(parse_color, 256)(i)?;
    Ok((i, Palette { colors }))
}

#[repr(C)]
#[derive(Debug, Serialize, Deserialize)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

fn parse_color<'a, Error: ParseError<&'a [u8]>>(i: &'a [u8]) -> IResult<&'a [u8], Color, Error> {
    Ok(parse_struct!(
        i,
        Color {
            red: be_u8,
            green: be_u8,
            blue: be_u8,
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_palette() {
        let file = std::fs::read(crate::palette_path()).unwrap();
        let (rest, palette) = palette_verbose(&file).unwrap();
        println!("frm: {:#?}, rest: {:?}", palette, rest);
    }

    #[test]
    fn test_palette_transmute() {
        use std::mem::size_of;
        assert_eq!(size_of::<Color>(), size_of::<(u8, u8, u8)>());
        assert_eq!(size_of::<[Color; 256]>(), size_of::<[(u8, u8, u8); 256]>());

        let colors = Palette {
            colors: vec![
                Color {
                    red: 10,
                    green: 20,
                    blue: 30,
                },
                Color {
                    red: 40,
                    green: 50,
                    blue: 60,
                },
                Color {
                    red: 70,
                    green: 80,
                    blue: 90,
                },
            ],
        };
        let tuples = colors.colors_tuples();
        assert_eq!(tuples, &[(10, 20, 30), (40, 50, 60), (70, 80, 90),]);
    }
}
