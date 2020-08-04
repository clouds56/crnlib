pub mod codec;
pub mod unpack;

use serde::{Serialize, Deserialize};
use anyhow::*;
use bincode::Options;

pub type Huffman = codec::Huffman<u32>;

#[derive(Debug, Copy, Clone, serde_repr::Serialize_repr, serde_repr::Deserialize_repr)]
#[repr(u8)]
pub enum Format {
  Dxt1 = 0, Dxt3, Dxt5,
  Dxt5CCxY, Dxt5xGxR, Dxt5xGBR, Dxt5AGBR,
  DxnXY, DxnYX,
  Dxt5A, Etc1,
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
    let chunk_encoding = codec.get_huffman().context("read chunk table")?;
    let color_endpoint_delta = codec.get_huffman().context("read color_endpoint table")?;
    let color_selector_delta = codec.get_huffman().context("read color_selector table")?;
    let alpha_endpoint_delta = codec.get_huffman().context("read alpha_endpoint table")?;
    let alpha_selector_delta = codec.get_huffman().context("read alpha_selector table")?;
    if !codec.is_complete() { bail!("extra bytes in codec") }
    let color_endpoints = self.get_color_endpoints(input).context("decode color_endpoints")?;
    let color_selectors = self.get_color_selectors(input).context("decode color_selectors")?;
    let alpha_endpoints = self.get_alpha_endpoints(input).context("decode alpha_endpoints")?;
    let alpha_selectors = self.get_alpha_selectors(input).context("decode alpha_selectors")?;
    Ok(Tables {
      chunk_encoding,
      color_endpoint: Table::new(color_endpoint_delta, color_endpoints),
      color_selector: Table::new(color_selector_delta, color_selectors),
      alpha_endpoint: Table::new(alpha_endpoint_delta, alpha_endpoints),
      alpha_selector: Table::new(alpha_selector_delta, alpha_selectors),
    })
  }

  pub fn get_color_endpoints(&self, input: &[u8]) -> Result<Vec<(u16, u16)>, Error> {
    let mut codec = if let Some(data) = self.get_palette_data(self.color_endpoints, input) {
      codec::Codec::new(&data)
    } else { return Ok(vec![]) };
    let dm1 = codec.get_huffman().context("color_endpoints_dm1")?;
    let dm2 = codec.get_huffman().context("color_endpoints_dm2")?;
    // println!("{:?} {:?}", dm1, dm2);
    let (mut a, mut b, mut c) = (0, 0, 0);
    let (mut d, mut e, mut f) = (0, 0, 0);
    let color_endpoints = (0..self.color_endpoints.count).map(|_i| {
      let da = dm1.next(&mut codec)? as u16; a = (a + da) & 0x1f;
      let db = dm2.next(&mut codec)? as u16; b = (b + db) & 0x3f;
      let dc = dm1.next(&mut codec)? as u16; c = (c + dc) & 0x1f;
      let dd = dm1.next(&mut codec)? as u16; d = (d + dd) & 0x1f;
      let de = dm2.next(&mut codec)? as u16; e = (e + de) & 0x3f;
      let df = dm1.next(&mut codec)? as u16; f = (f + df) & 0x1f;
      Ok::<_, Error>((c | (b << 5) | (a << 11), f | (e << 5) | (d << 11)))
    }).collect::<Result<Vec<_>, _>>()?;
    if !codec.is_complete() { bail!("extra bytes in codec") }
    Ok(color_endpoints)
  }

  pub fn get_alpha_endpoints(&self, input: &[u8]) -> Result<Vec<(u8, u8)>, Error> {
    let mut codec = if let Some(data) = self.get_palette_data(self.alpha_endpoints, input) {
      codec::Codec::new(&data)
    } else { return Ok(vec![]) };
    let dm = codec.get_huffman().context("alpha_endpoints_dm1")?;
    // println!("{:?}", dm);
    let (mut a, mut b) = (0, 0);
    let color_endpoints = (0..self.alpha_endpoints.count).map(|_i| {
      let da = dm.next(&mut codec)?; a = (a as u32 + da) as u8;
      let db = dm.next(&mut codec)?; b = (b as u32 + db) as u8;
      Ok::<_, Error>((a, b))
    }).collect::<Result<Vec<_>, _>>()?;
    if !codec.is_complete() { bail!("extra bytes in codec") }
    Ok(color_endpoints)
  }

  pub fn get_color_selectors(&self, input: &[u8]) -> Result<Vec<[u8; 4]>, Error> {
    let mut codec = if let Some(data) = self.get_palette_data(self.color_selectors, input) {
      codec::Codec::new(&data)
    } else { return Ok(vec![]) };
    let dm = codec.get_huffman().context("color_selectors_dm")?;
    // println!("{:?}", dm);

    let mut x = [0; 8];
    let mut y = [0; 8];

    const C: [u8; 4] = [0, 2, 3, 1]; // DXT1

    let color_selectors = (0..self.color_selectors.count).map(|_i| {
      for (x, y) in &mut x.iter_mut().zip(&mut y) {
        let d = dm.next(&mut codec)? as i32;
        *x = ((*x as i32 + d % 7 - 3) & 3) as usize;
        *y = ((*y as i32 + d / 7 - 3) & 3) as usize;
      }

      let result = [
        C[x[0]] | (C[y[0]] << 2) | (C[x[1]] << 4) | (C[y[1]] << 6),
        C[x[2]] | (C[y[2]] << 2) | (C[x[3]] << 4) | (C[y[3]] << 6),
        C[x[4]] | (C[y[4]] << 2) | (C[x[5]] << 4) | (C[y[5]] << 6),
        C[x[6]] | (C[y[6]] << 2) | (C[x[7]] << 4) | (C[y[7]] << 6),
      ];
      Ok::<_, Error>(result)
    }).collect::<Result<Vec<_>, _>>()?;
    if !codec.is_complete() { bail!("extra bytes in codec") }

    Ok(color_selectors)
  }

  pub fn get_alpha_selectors(&self, input: &[u8]) -> Result<Vec<[u8; 6]>, Error> {
    let mut codec = if let Some(data) = self.get_palette_data(self.alpha_selectors, input) {
      codec::Codec::new(&data)
    } else { return Ok(vec![]) };
    let dm = codec.get_huffman().context("alpha_selectors_dm")?;
    // println!("{:?}", dm);

    let mut x = [0; 8];
    let mut y = [0; 8];

    const C: [u16; 8] = [0, 2, 3, 4, 5, 6, 7, 1]; // DXT5

    let alpha_selectors = (0..self.alpha_selectors.count).map(|_i| {
      use bitvec::{slice::BitSlice, order::Msb0, fields::BitField};
      let mut s = [0u8; 6];
      let s_bits = BitSlice::<Msb0, u8>::from_slice_mut(&mut s);
      let s_len = s_bits.len();
      for (j, (x, y)) in &mut x.iter_mut().zip(&mut y).enumerate() {
        let d = dm.next(&mut codec)? as i32;
        *x = ((*x as i32 + d % 15 - 7) & 7) as usize;
        *y = ((*y as i32 + d / 15 - 7) & 7) as usize;
        s_bits[s_len-j*6-3..s_len-j*6-0].store_be(C[*x]);
        s_bits[s_len-j*6-6..s_len-j*6-3].store_be(C[*y]);
      }
      s.reverse();
      Ok::<_, Error>(s)
    }).collect::<Result<Vec<_>, Error>>()?;
    if !codec.is_complete() { bail!("extra bytes in codec") }

    Ok(alpha_selectors)
  }

  pub fn get_level_info(&self, idx: usize) -> Option<(u16, u16)> {
    if idx < self.level_count as usize {
      let width = 1.max(self.width >> idx);
      let height = 1.max(self.height >> idx);
      (width, height).into()
    } else { None }
  }

  pub fn unpack_level(&self, tables: &Tables, input: &[u8], idx: usize) -> Result<Vec<u8>, Error> {
    use crate::unpack::Unpack;
    let mut codec = if let Some(data) = self.get_level_data(input, idx) {
      codec::Codec::new(data)
    } else { bail!("level out of index") };
    let width = 1.max(self.width >> idx);
    let height = 1.max(self.height >> idx);
    match self.format {
      Format::Dxt1 => unimplemented!("
        unpack::Dxt1::unpack(tables, &mut codec, width, height, self.face_count)
      "),
      Format::Dxt5 | Format::Dxt5AGBR | Format::Dxt5CCxY | Format::Dxt5xGBR | Format::Dxt5xGxR =>
        unpack::Dxt5::unpack(tables, &mut codec, width, height, self.face_count),
      Format::Dxt5A => unimplemented!("
        unpack::Dxt5A::unpack(tables, &mut codec, width, height, self.face_count)
      "),
      Format::DxnXY | Format::DxnYX => unimplemented!("
        unpack::Dxn::unpack(tables, &mut codec, width, height, self.face_count)
      "),
      Format::Dxt3 | Format::Etc1 | Format::Invalid => bail!("unsupported format {:?}", self.format),
    }
  }
}

#[derive(Debug)]
pub struct Tables {
  pub chunk_encoding: Huffman,

  pub color_endpoint: Table<(u16, u16)>,
  pub color_selector: Table<[u8; 4]>,
  pub alpha_endpoint: Table<(u8, u8)>,
  pub alpha_selector: Table<[u8; 6]>,
}

#[derive(Debug)]
pub struct Table<T> {
  pub delta: Huffman,
  pub entries: Vec<T>,
}

impl<T: Copy> Table<T> {
  fn new(delta: Huffman, entries: Vec<T>) -> Self {
    Self { delta, entries }
  }
  #[inline]
  fn truncate(idx: usize, max: usize) -> usize {
    if idx < max { idx } else { idx-max }
  }
  pub fn next(&self, codec: &mut codec::Codec, idx: &mut usize) -> Result<T, Error> {
    let delta = self.delta.next(codec)? as usize;
    *idx = Self::truncate(*idx + delta, self.entries.len());
    Ok(self.entries[*idx])
  }
}

#[test]
fn test_file() {
  use std::io::prelude::*;
  let sample = "samples/test.crn";
  assert_eq!(Header::fixed_size(), Header::serialize_option()
    .serialized_size(&Header::default()).expect("header size") as usize);
  let mut file = std::fs::File::open(sample).expect("open sample crn file");
  let mut buffer = Vec::new();
  file.read_to_end(&mut buffer).expect("read crn file");
  let header = Header::parse(&buffer).expect("parse");
  println!("header: {:x?}", header);
  assert_eq!(header.header_size as usize, Header::fixed_size() + 4*header.level_count as usize);
  assert!(header.check_crc(&buffer));

  let tables = header.get_table(&buffer).expect("read table");
  println!("table: {:x?}", tables);
  let level0 = header.unpack_level(&tables, &buffer, 0).expect("unpack");
  println!("{:02x?}", level0);
  header.unpack_level(&tables, &buffer, header.level_count as usize - 1).expect("unpack");

  use image::ImageDecoder;
  let (width0, height0) = header.get_level_info(0).expect("get level info");
  assert_eq!((width0, height0), (header.width, header.height));
  let decoder = image::dxt::DxtDecoder::new(std::io::Cursor::new(&level0), width0 as u32, height0 as u32, image::dxt::DXTVariant::DXT5).expect("new image");
  let mut raw = vec![0; decoder.total_bytes() as usize];
  let color_type = decoder.color_type();
  decoder.read_image(&mut raw).expect("decode dxt");
  let f = std::fs::File::create(std::path::Path::new(sample).with_extension("tga")).expect("create sample tga file");
  let encoder = image::tga::TgaEncoder::new(f);
  encoder.encode(&raw, width0 as u32, height0 as u32, color_type).expect("encode tga");
}
