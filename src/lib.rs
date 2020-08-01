pub mod codec;

use serde::{Serialize, Deserialize};
use anyhow::*;
use bincode::Options;

type Huffman = codec::Huffman<u32>;

#[derive(Debug, Copy, Clone, serde_repr::Serialize_repr, serde_repr::Deserialize_repr)]
#[repr(u8)]
pub enum Format {
  Dxt1 = 0, Dxt1A, Dxt3, Dxt5, Dxt5A, DxnXY, DxnYX,
  Invalid = 0xff,
}
impl Default for Format {
  fn default() -> Self {
    Format::Invalid
  }
}

pub mod be_u24 {
  use serde::{Serialize, Serializer, Deserialize, Deserializer};
  pub fn deserialize<'de, D>(deserializer: D) -> Result<u32, D::Error> where D: Deserializer<'de> {
    <[u8; 3]>::deserialize(deserializer).map(|x| (x[0] as u32) << 16 | (x[1] as u32) << 8 | x[2] as u32)
  }

  pub fn serialize<S>(i: &u32, serializer: S) -> Result<S::Ok, S::Error> where S: Serializer {
    [(i >> 16 & 0xff) as u8 , (i >> 8 & 0xff) as u8, (i & 0xff) as u8].serialize(serializer)
  }
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct Palette {
  #[serde(with = "be_u24")]
  pub offset: u32,
  #[serde(with = "be_u24")]
  pub size: u32,
  pub count: u16,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Header {
  pub magic: [u8; 2],
  pub header_size: u16,
  pub header_crc16: u16,
  pub file_size: u32,
  pub data_crc16: u16,

  pub width: u16,
  pub height: u16,
  pub level_count: u8,
  pub face_count: u8, // 1 or 6
  pub format: Format, // u8
  pub flags: u16,

  pub reserved: u32,
  pub userdata: [u32; 2],

  pub color_endpoints: Palette,
  pub color_selectors: Palette,
  pub alpha_endpoints: Palette,
  pub alpha_selectors: Palette,

  pub table_size: u16,
  #[serde(with = "be_u24")]
  pub table_offset: u32,

  #[serde(skip)]
  pub level_offset: Vec<u32>,
}

impl Header {
  fn serialize_option() -> impl bincode::Options {
    bincode::config::DefaultOptions::new()
      .allow_trailing_bytes()
      .with_fixint_encoding()
      .with_big_endian()
  }
  pub fn parse(input: &[u8]) -> Result<Self, Error> {
    let mut result: Header = Self::serialize_option()
      .deserialize(input)?;
    result.level_offset = (0..result.level_count as usize).map(|i|
      Self::serialize_option().deserialize::<u32>(&input[Self::fixed_size() + 4*i..])).collect::<Result<_, _>>()?;
    Ok(result)
  }

  pub fn fixed_size() -> usize {
    33 + 8*4 + 5
  }

  pub fn crc16(init: u16, input: &[u8]) -> u16 {
    input.iter().fold(!init, |v, &c| {
      let x = c ^ (v >> 8) as u8;
      let x = (x ^ (x >> 4)) as u16;
      (v << 8) ^ (x << 12) ^ (x << 5) ^ x
    })
  }

  pub fn crc16_poly(init: u16, poly: u16, input: &[u8]) -> u16 {
    input.iter().fold(!init, |v, &c| {
      (0..8).fold(v ^ c as u16, |v, _| {
        if v & 1 == 1 { (v >> 1) ^ poly} else { v >> 1 }
      })
    })
  }

  pub fn check_crc(&self, input: &[u8]) -> bool {
    self.header_size as usize == Header::fixed_size() + 4*self.level_count as usize &&
    self.file_size as usize == input.len() &&
    self.header_crc16 == !Self::crc16(0, &input[6..self.header_size as usize]) &&
    self.data_crc16 == !Self::crc16(0, &input[self.header_size as usize..])
  }

  pub fn block_size(&self) -> usize {
    match self.format {
      Format::Dxt1 | Format::Dxt5A => 8,
      _ => 16,
    }
  }

  pub fn get_level_data<'a>(&self, input: &'a [u8], idx: usize) -> Option<&'a [u8]> {
    let start = *self.level_offset.get(idx)? as usize;
    let end = self.level_offset.get(idx+1).cloned().unwrap_or(self.file_size) as usize;
    Some(&input[start..end])
  }

  fn get_table_data<'a>(&self, input: &'a [u8]) -> &'a [u8] {
    let start = self.table_offset as usize;
    let end = start + self.table_size as usize;
    &input[start..end]
  }

  fn get_palette_data<'a>(&self, palette: Palette, input: &'a [u8]) -> Option<&'a [u8]> {
    if palette.count == 0 { return None }
    let start = palette.offset as usize;
    let end = start + palette.size as usize;
    Some(&input[start..end])
  }

  pub fn get_table(&self, input: &[u8]) -> Result<Tables, Error> {
    let mut codec = codec::Codec::new(self.get_table_data(input));
    let chunk = codec.get_huffman().context("read chunk table")?;
    let color_endpoint = if self.color_endpoints.count != 0 {
      codec.get_huffman().context("read color_endpoint table")?.into()
    } else { None };
    let color_selector = if self.color_selectors.count != 0 {
      codec.get_huffman().context("read color_selector table")?.into()
    } else { None };
    let alpha_endpoint = if self.alpha_endpoints.count != 0 {
      codec.get_huffman().context("read alpha_endpoint table")?.into()
    } else { None };
    let alpha_selector = if self.alpha_selectors.count != 0 {
      codec.get_huffman().context("read alpha_selector table")?.into()
    } else { None };
    if !codec.is_complete() { bail!("extra bytes in codec") }
    Ok(Tables { chunk, color_endpoint, color_selector, alpha_endpoint, alpha_selector })
  }

  pub fn get_color_endpoints(&self, input: &[u8]) -> Result<Option<Vec<u32>>, Error> {
    let mut codec = if let Some(data) = self.get_palette_data(self.color_endpoints, input) {
      codec::Codec::new(&data)
    } else { return Ok(None) };
    let dm1 = codec.get_huffman().context("color_endpoints_dm1")?;
    let dm2 = codec.get_huffman().context("color_endpoints_dm2")?;
    // println!("{:?} {:?}", dm1, dm2);
    let (mut a, mut b, mut c) = (0, 0, 0);
    let (mut d, mut e, mut f) = (0, 0, 0);
    let color_endpoints = (0..self.color_endpoints.count).map(|_i| {
      let da = dm1.next(&mut codec)?; a = (a + da) & 0x1f;
      let db = dm2.next(&mut codec)?; b = (b + db) & 0x3f;
      let dc = dm1.next(&mut codec)?; c = (c + dc) & 0x1f;
      let dd = dm1.next(&mut codec)?; d = (d + dd) & 0x1f;
      let de = dm2.next(&mut codec)?; e = (e + de) & 0x3f;
      let df = dm1.next(&mut codec)?; f = (f + df) & 0x1f;
      Ok::<_, Error>(c | (b << 5) | (a << 11) | (f << 16) | (e << 21) | (d << 27))
    }).collect::<Result<Vec<_>, _>>()?;
    if !codec.is_complete() { bail!("extra bytes in codec") }
    Ok(Some(color_endpoints))
  }

  pub fn get_alpha_endpoints(&self, input: &[u8]) -> Result<Option<Vec<u16>>, Error> {
    let mut codec = if let Some(data) = self.get_palette_data(self.alpha_endpoints, input) {
      codec::Codec::new(&data)
    } else { return Ok(None) };
    let dm = codec.get_huffman().context("alpha_endpoints_dm1")?;
    // println!("{:?}", dm);
    let (mut a, mut b) = (0, 0);
    let color_endpoints = (0..self.alpha_endpoints.count).map(|_i| {
      let da = dm.next(&mut codec)?; a = (a + da as u16) & 0xff;
      let db = dm.next(&mut codec)?; b = (b + db as u16) & 0xff;
      Ok::<_, Error>(a | (b << 8))
    }).collect::<Result<Vec<_>, _>>()?;
    if !codec.is_complete() { bail!("extra bytes in codec") }
    Ok(Some(color_endpoints))
  }

  pub fn get_color_selectors(&self, input: &[u8]) -> Result<Option<Vec<u32>>, Error> {
    let mut codec = if let Some(data) = self.get_palette_data(self.color_selectors, input) {
      codec::Codec::new(&data)
    } else { return Ok(None) };
    let dm = codec.get_huffman().context("color_selectors_dm")?;
    // println!("{:?}", dm);

    let mut x = [0; 8];
    let mut y = [0; 8];

    const C: [u32; 4] = [0, 2, 3, 1]; // DXT1

    let color_selectors = (0..self.color_selectors.count).map(|_i| {
      for (x, y) in &mut x.iter_mut().zip(&mut y) {
        let d = dm.next(&mut codec)? as i32;
        *x = ((*x as i32 + d % 15 - 7) & 3) as usize;
        *y = ((*y as i32 + d / 15 - 7) & 3) as usize;
      }

      let result =
      (C[x[0]]      ) | (C[y[0]] <<  2) | (C[x[1]] <<  4) | (C[y[1]] <<  6) |
      (C[x[2]] <<  8) | (C[y[2]] << 10) | (C[x[3]] << 12) | (C[y[3]] << 14) |
      (C[x[4]] << 16) | (C[y[4]] << 18) | (C[x[5]] << 20) | (C[y[5]] << 22) |
      (C[x[6]] << 24) | (C[y[6]] << 26) | (C[x[7]] << 28) | (C[y[7]] << 30);
      Ok::<_, Error>(result)
    }).collect::<Result<Vec<_>, _>>()?;
    if !codec.is_complete() { bail!("extra bytes in codec") }

    Ok(Some(color_selectors))
  }

  pub fn get_alpha_selectors(&self, input: &[u8]) -> Result<Option<Vec<(u16, u16, u16)>>, Error> {
    let mut codec = if let Some(data) = self.get_palette_data(self.alpha_selectors, input) {
      codec::Codec::new(&data)
    } else { return Ok(None) };
    let dm = codec.get_huffman().context("alpha_selectors_dm")?;
    // println!("{:?}", dm);

    let mut x = [0; 8];
    let mut y = [0; 8];

    const C: [u16; 8] = [0, 2, 3, 4, 5, 6, 7, 1]; // DXT5

    let alpha_selectors = (0..self.alpha_selectors.count).map(|_i| {
      for (x, y) in &mut x.iter_mut().zip(&mut y) {
        let d = dm.next(&mut codec)? as i32;
        *x = ((*x as i32 + d % 15 - 7) & 3) as usize;
        *y = ((*y as i32 + d / 15 - 7) & 3) as usize;
      }

      Ok::<_, Error>((
        (C[x[0]]      ) | (C[y[0]] <<  3) | (C[x[1]] <<  6) | (C[y[1]] <<  9) | (C[x[2]] << 12) | (C[y[2]] << 15),
        (C[y[2]] >>  1) | (C[x[3]] <<  2) | (C[y[3]] <<  5) | (C[x[4]] <<  8) | (C[y[4]] << 11) | (C[x[5]] << 14),
        (C[x[5]] >>  2) | (C[y[5]] <<  1) | (C[x[6]] <<  4) | (C[y[6]] <<  7) | (C[x[7]] << 10) | (C[y[7]] << 13),
      ))
    }).collect::<Result<Vec<_>, Error>>()?;
    if !codec.is_complete() { bail!("extra bytes in codec") }

    Ok(Some(alpha_selectors))
  }
}

#[derive(Debug)]
pub struct Tables {
  pub chunk: Huffman,
  pub color_endpoint: Option<Huffman>,
  pub color_selector: Option<Huffman>,
  pub alpha_endpoint: Option<Huffman>,
  pub alpha_selector: Option<Huffman>,
}

#[test]
fn test_header() {
  use std::io::prelude::*;
  let sample = "samples/test.crn";
  assert_eq!(Header::fixed_size(), Header::serialize_option()
    .serialized_size(&Header::default()).expect("header size") as usize);
  let mut file = std::fs::File::open(sample).expect("open sample crn file");
  let mut buffer = Vec::new();
  file.read_to_end(&mut buffer).expect("read crn file");
  let h = Header::parse(&buffer).expect("parse");
  println!("header: {:x?}", h);
  assert_eq!(h.header_size as usize, Header::fixed_size() + 4*h.level_count as usize);
  assert!(h.check_crc(&buffer));

  let _table = h.get_table(&buffer).expect("read table");
  // println!("table: {:?}", table);
  let _color_endpoints = h.get_color_endpoints(&buffer).expect("read color_endpoints");
  println!("color_endpoints: {:x?}", _color_endpoints);
  let _color_selectors = h.get_color_selectors(&buffer).expect("read color_selectors");
  println!("color_selectors: {:x?}", _color_selectors);
  let _alpha_endpoints = h.get_alpha_endpoints(&buffer).expect("read alpha_endpoints");
  println!("alpha_endpoints: {:x?}", _alpha_endpoints);
  let _alpha_selectors = h.get_alpha_selectors(&buffer).expect("read alpha_selectors");
  println!("alpha_selectors: {:x?}", _alpha_selectors);
}
