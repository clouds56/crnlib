use std::io::prelude::*;
use anyhow::*;
use serde::{Serialize, Deserialize};
use crate::{Tables, Huffman, codec::Codec};

pub trait Block: Serialize {
  const BLOCK_SIZE: usize;
  fn write_to<W: Write>(&self, mut w: W) -> std::io::Result<()> {
    use bincode::Options;
    let bin = bincode::config::DefaultOptions::new()
      .with_fixint_encoding()
      .with_little_endian()
      .serialize(self)
      .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    w.write(&bin)?;
    Ok(())
  }
}

pub trait Unpack {
  fn unpack(tables: &Tables, codec: &mut Codec, width: u16, height: u16, face: u8) -> Result<Vec<u8>, Error>;
  fn next_tile_idx(codec: &mut Codec, encoding: &Huffman, tile_bits: &mut u32) -> Result<(usize, [usize; 4]), Error> {
    if *tile_bits == 1 {
      *tile_bits = encoding.next(codec).context("read chunk encoding bits")? | 512;
    }

    let tile_index = *tile_bits as usize & 7;
    *tile_bits >>= 3;
    Ok((Self::COUNT_TILES[tile_index], Self::TILES[tile_index]))
  }

  const TRUNK_SIZE: usize = 2;

  const COUNT_TILES: [usize; 8] = [ 1, 2, 2, 3, 3, 3, 3, 4 ];
  const TILES: [[usize; 4]; 8] = [
    [ 0, 0, 0, 0 ],
    [ 0, 0, 1, 1 ], [ 0, 1, 0, 1 ],
    [ 0, 0, 1, 2 ], [ 1, 2, 0, 0 ],
    [ 0, 1, 0, 2 ], [ 1, 0, 2, 0 ],
    [ 0, 1, 2, 3 ]
  ];
}



#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Dxt1 {
  pub color_endpoint: (u16, u16),
  pub color_selector: [u8; 4],
}

impl Block for Dxt1 {
  const BLOCK_SIZE: usize = 8;
}
impl Unpack for Dxt1 {
  fn unpack(tables: &Tables, codec: &mut Codec, width: u16, height: u16, face: u8) -> Result<Vec<u8>, Error> {
    let block_x = (width + 3) / 4;
    let block_y = (height + 3) / 4;
    let chunk_x = (block_x + 1) as usize / Self::TRUNK_SIZE;
    let chunk_y = (block_y + 1) as usize / Self::TRUNK_SIZE;

    let mut tile_bits = 1u32;

    let mut color_endpoint_index = 0;
    let mut color_selector_index = 0;

    let pitch = block_x as usize * Self::BLOCK_SIZE;

    let mut result = vec![0u8; block_y as usize * pitch];
    let mut cursor = std::io::Cursor::new(&mut result[..]);

    for _f in 0..face {
      // let mut row = Vec::new();
      for y in 0..chunk_y {
        let skip_y = y == (chunk_y - 1) && block_y & 1 == 1;
        let xrange: Box<dyn Iterator<Item=_>> = if y & 1 == 1 { Box::new((0..chunk_x).rev()) } else { Box::new(0..chunk_x) };
        for x in xrange {
          let skip_x = block_x & 1 == 1 && x == (chunk_x - 1);
          let mut color_endpoints = [(0, 0); 4];

          let (tiles_count, tiles) = Self::next_tile_idx(codec, &tables.chunk_encoding, &mut tile_bits)?;

          for i in 0..tiles_count {
            color_endpoints[i] = tables.color_endpoint.next(codec, &mut color_endpoint_index).context("read color_endpoint_delta")?;
          }
          for (i, &tile) in tiles.iter().enumerate() {
            let color_selector = tables.color_selector.next(codec, &mut color_selector_index).context("read color_selector_delta")?;

            // println!("{:x?}", (delta0, delta1, alpha_selector_index, color_selector_index, tables.color_selectors[color_selector_index]));
            if !skip_x && !skip_y {
              if i % Self::TRUNK_SIZE == 0 {
                let pos = (y * Self::TRUNK_SIZE + i / Self::TRUNK_SIZE) * pitch + x * Self::BLOCK_SIZE * Self::TRUNK_SIZE;
                // println!("seek {}x{} + {} => {:x}", x, y, i, pos);
                cursor.seek(std::io::SeekFrom::Start(pos as _)).expect("seek");
              }
              Dxt1 {
                color_endpoint: color_endpoints[tile],
                color_selector,
              }.write_to(&mut cursor).context("write block")?;
            }
          }
        }
      }
    }
    if !codec.is_complete() { bail!("extra bytes in codec") }
    Ok(result)
  }
}


#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Dxt5 {
  pub alpha_endpoint: (u8, u8),
  pub alpha_selector: [u8; 6],
  pub color_endpoint: (u16, u16),
  pub color_selector: [u8; 4],
}

impl Block for Dxt5 {
  const BLOCK_SIZE: usize = 16;
}
impl Unpack for Dxt5 {
  fn unpack(tables: &Tables, codec: &mut Codec, width: u16, height: u16, face: u8) -> Result<Vec<u8>, Error> {
    let block_x = (width + 3) / 4;
    let block_y = (height + 3) / 4;
    let chunk_x = (block_x + 1) as usize / Self::TRUNK_SIZE;
    let chunk_y = (block_y + 1) as usize / Self::TRUNK_SIZE;

    let mut tile_bits = 1u32;

    let mut color_endpoint_index = 0;
    let mut color_selector_index = 0;
    let mut alpha_endpoint_index = 0;
    let mut alpha_selector_index = 0;

    let pitch = block_x as usize * Self::BLOCK_SIZE;

    let mut result = vec![0u8; block_y as usize * pitch];
    let mut cursor = std::io::Cursor::new(&mut result[..]);

    for _f in 0..face {
      // let mut row = Vec::new();
      for y in 0..chunk_y {
        let skip_y = y == (chunk_y - 1) && block_y & 1 == 1;
        let xrange: Box<dyn Iterator<Item=_>> = if y & 1 == 1 { Box::new((0..chunk_x).rev()) } else { Box::new(0..chunk_x) };
        for x in xrange {
          let skip_x = block_x & 1 == 1 && x == (chunk_x - 1);
          let mut color_endpoints = [(0, 0); 4];
          let mut alpha_endpoints = [(0, 0); 4];

          let (tiles_count, tiles) = Self::next_tile_idx(codec, &tables.chunk_encoding, &mut tile_bits)?;

          for i in 0..tiles_count {
            alpha_endpoints[i] = tables.alpha_endpoint.next(codec, &mut alpha_endpoint_index).context("read alpha_endpoint_delta")?;
          }

          for i in 0..tiles_count {
            color_endpoints[i] = tables.color_endpoint.next(codec, &mut color_endpoint_index).context("read color_endpoint_delta")?;
          }

          // println!("tile: {:x?}", tiles);
          for (i, &tile) in tiles.iter().enumerate() {
            let alpha_selector = tables.alpha_selector.next(codec, &mut alpha_selector_index).context("read alpha_selector_delta")?;
            let color_selector = tables.color_selector.next(codec, &mut color_selector_index).context("read color_selector_delta")?;

            // println!("{:x?}", (delta0, delta1, alpha_selector_index, color_selector_index, tables.color_selectors[color_selector_index]));
            if !skip_x && !skip_y {
              if i % Self::TRUNK_SIZE == 0 {
                let pos = (y * Self::TRUNK_SIZE + i / Self::TRUNK_SIZE) * pitch + x * Self::BLOCK_SIZE * Self::TRUNK_SIZE;
                // println!("seek {}x{} + {} => {:x}", x, y, i, pos);
                cursor.seek(std::io::SeekFrom::Start(pos as _)).expect("seek");
              }
              Dxt5 {
                alpha_endpoint: alpha_endpoints[tile],
                alpha_selector,
                color_endpoint: color_endpoints[tile],
                color_selector,
              }.write_to(&mut cursor).context("write block")?;
            }
          }
        }
      }
    }
    if !codec.is_complete() { bail!("extra bytes in codec") }
    Ok(result)
  }
}


#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Dxt5A {
  pub alpha_endpoint: (u8, u8),
  pub alpha_selector: [u8; 6],
}

impl Block for Dxt5A {
  const BLOCK_SIZE: usize = 8;
}
impl Unpack for Dxt5A {
  fn unpack(tables: &Tables, codec: &mut Codec, width: u16, height: u16, face: u8) -> Result<Vec<u8>, Error> {
    let block_x = (width + 3) / 4;
    let block_y = (height + 3) / 4;
    let chunk_x = (block_x + 1) as usize / Self::TRUNK_SIZE;
    let chunk_y = (block_y + 1) as usize / Self::TRUNK_SIZE;

    let mut tile_bits = 1u32;

    let mut alpha_endpoint_index = 0;
    let mut alpha_selector_index = 0;

    let pitch = block_x as usize * Self::BLOCK_SIZE;

    let mut result = vec![0u8; block_y as usize * pitch];
    let mut cursor = std::io::Cursor::new(&mut result[..]);

    for _f in 0..face {
      // let mut row = Vec::new();
      for y in 0..chunk_y {
        let skip_y = y == (chunk_y - 1) && block_y & 1 == 1;
        let xrange: Box<dyn Iterator<Item=_>> = if y & 1 == 1 { Box::new((0..chunk_x).rev()) } else { Box::new(0..chunk_x) };
        for x in xrange {
          let skip_x = block_x & 1 == 1 && x == (chunk_x - 1);
          let mut alpha_endpoints = [(0, 0); 4];

          let (tiles_count, tiles) = Self::next_tile_idx(codec, &tables.chunk_encoding, &mut tile_bits)?;

          for i in 0..tiles_count {
            alpha_endpoints[i] = tables.alpha_endpoint.next(codec, &mut alpha_endpoint_index).context("read alpha_endpoint_delta")?;
          }
          for (i, &tile) in tiles.iter().enumerate() {
            let alpha_selector = tables.alpha_selector.next(codec, &mut alpha_selector_index).context("read alpha_selector_delta")?;

            // println!("{:x?}", (delta0, delta1, alpha_selector_index, color_selector_index, tables.color_selectors[color_selector_index]));
            if !skip_x && !skip_y {
              if i % Self::TRUNK_SIZE == 0 {
                let pos = (y * Self::TRUNK_SIZE + i / Self::TRUNK_SIZE) * pitch + x * Self::BLOCK_SIZE * Self::TRUNK_SIZE;
                // println!("seek {}x{} + {} => {:x}", x, y, i, pos);
                cursor.seek(std::io::SeekFrom::Start(pos as _)).expect("seek");
              }
              Dxt5A {
                alpha_endpoint: alpha_endpoints[tile],
                alpha_selector,
              }.write_to(&mut cursor).context("write block")?;
            }
          }
        }
      }
    }
    if !codec.is_complete() { bail!("extra bytes in codec") }
    Ok(result)
  }
}


#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Dxn {
  pub alpha0_endpoint: (u8, u8),
  pub alpha0_selector: [u8; 6],
  pub alpha1_endpoint: (u8, u8),
  pub alpha1_selector: [u8; 6],
}

impl Block for Dxn {
  const BLOCK_SIZE: usize = 16;
}
impl Unpack for Dxn {
  fn unpack(tables: &Tables, codec: &mut Codec, width: u16, height: u16, face: u8) -> Result<Vec<u8>, Error> {
    let block_x = (width + 3) / 4;
    let block_y = (height + 3) / 4;
    let chunk_x = (block_x + 1) as usize / Self::TRUNK_SIZE;
    let chunk_y = (block_y + 1) as usize / Self::TRUNK_SIZE;

    let mut tile_bits = 1u32;

    let mut alpha0_endpoint_index = 0;
    let mut alpha0_selector_index = 0;
    let mut alpha1_endpoint_index = 0;
    let mut alpha1_selector_index = 0;

    let pitch = block_x as usize * Self::BLOCK_SIZE;

    let mut result = vec![0u8; block_y as usize * pitch];
    let mut cursor = std::io::Cursor::new(&mut result[..]);

    for _f in 0..face {
      // let mut row = Vec::new();
      for y in 0..chunk_y {
        let skip_y = y == (chunk_y - 1) && block_y & 1 == 1;
        let xrange: Box<dyn Iterator<Item=_>> = if y & 1 == 1 { Box::new((0..chunk_x).rev()) } else { Box::new(0..chunk_x) };
        for x in xrange {
          let skip_x = block_x & 1 == 1 && x == (chunk_x - 1);
          let mut alpha0_endpoints = [(0, 0); 4];
          let mut alpha1_endpoints = [(0, 0); 4];

          let (tiles_count, tiles) = Self::next_tile_idx(codec, &tables.chunk_encoding, &mut tile_bits)?;

          for i in 0..tiles_count {
            alpha0_endpoints[i] = tables.alpha_endpoint.next(codec, &mut alpha0_endpoint_index).context("read alpha0_endpoint_delta")?;
          }
          for i in 0..tiles_count {
            alpha1_endpoints[i] = tables.alpha_endpoint.next(codec, &mut alpha1_endpoint_index).context("read alpha1_endpoint_delta")?;
          }
          for (i, &tile) in tiles.iter().enumerate() {
            let alpha0_selector = tables.alpha_selector.next(codec, &mut alpha0_selector_index).context("read alpha0_selector_delta")?;
            let alpha1_selector = tables.alpha_selector.next(codec, &mut alpha1_selector_index).context("read alpha1_selector_delta")?;

            // println!("{:x?}", (delta0, delta1, alpha_selector_index, color_selector_index, tables.color_selectors[color_selector_index]));
            if !skip_x && !skip_y {
              if i % Self::TRUNK_SIZE == 0 {
                let pos = (y * Self::TRUNK_SIZE + i / Self::TRUNK_SIZE) * pitch + x * Self::BLOCK_SIZE * Self::TRUNK_SIZE;
                // println!("seek {}x{} + {} => {:x}", x, y, i, pos);
                cursor.seek(std::io::SeekFrom::Start(pos as _)).expect("seek");
              }
              Dxn {
                alpha0_endpoint: alpha0_endpoints[tile],
                alpha0_selector,
                alpha1_endpoint: alpha1_endpoints[tile],
                alpha1_selector,
              }.write_to(&mut cursor).context("write block")?;
            }
          }
        }
      }
    }
    if !codec.is_complete() { bail!("extra bytes in codec") }
    Ok(result)
  }
}

#[test]
fn test_constant() {
  assert_eq!(Dxt5::TILES.len(), Dxt5::COUNT_TILES.len());
  assert_eq!(Dxt5::TILES[0].len(), Dxt5::TRUNK_SIZE * Dxt5::TRUNK_SIZE);

  use bincode::Options;
  let option = || bincode::config::DefaultOptions::new()
    .with_fixint_encoding()
    .with_little_endian();
  assert_eq!(option().serialized_size(&Dxt1::default()).unwrap(), Dxt1::BLOCK_SIZE as u64);
  assert_eq!(option().serialized_size(&Dxt5::default()).unwrap(), Dxt5::BLOCK_SIZE as u64);
  assert_eq!(option().serialized_size(&Dxt5A::default()).unwrap(), Dxt5A::BLOCK_SIZE as u64);
  assert_eq!(option().serialized_size(&Dxn::default()).unwrap(), Dxn::BLOCK_SIZE as u64);

  assert_eq!(option().serialize(&Dxt5 {
    alpha_endpoint: (0x17, 0x18),
    alpha_selector: [0x20, 0x21, 0x22, 0x23, 0x24, 0x25],
    color_endpoint: (0x3234, 0x3537),
    color_selector: [0x49, 0x48, 0x47, 0x46],
  }).unwrap(), &[
    0x17, 0x18,
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25,
    0x34, 0x32, 0x37, 0x35,
    0x49, 0x48, 0x47, 0x46]);
}
