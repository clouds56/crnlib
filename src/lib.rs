pub mod codec;

use serde::{Serialize, Deserialize};
use anyhow::*;
use bincode::Options;

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

#[derive(Debug, Default, Serialize, Deserialize)]
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

  pub fn get_table_data<'a>(&self, input: &'a [u8]) -> &'a [u8] {
    let start = self.table_offset as usize;
    let end = start + self.table_size as usize;
    &input[start..end]
  }
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

  let table = h.get_table_data(&buffer);
  let mut codec = codec::Codec::new(table);
  println!("{:?}", codec.decode());
}
