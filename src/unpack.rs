use anyhow::*;
use std::io::prelude::*;
use crate::{Tables, codec::Codec};

pub struct Dxt5;

pub trait Unpack {
  fn unpack(tables: &Tables, codec: &mut Codec, width: u16, height: u16, face: u8) -> Result<Vec<u8>, Error>;
}

impl Unpack for Dxt5 {
  fn unpack(tables: &Tables, codec: &mut Codec, width: u16, height: u16, face: u8) -> Result<Vec<u8>, Error> {
    let block_x = (width + 3) / 4;
    let block_y = (height + 3) / 4;
    let chunk_x = (block_x + 1) as usize / 2;
    let chunk_y = (block_y + 1) as usize / 2;

    let mut chunk_encoding_bits = 1u32;

    const COUNT_TILES: [usize; 8] = [ 1, 2, 2, 3, 3, 3, 3, 4 ];
    const TILES: [[usize; 4]; 8] = [
      [ 0, 0, 0, 0 ],
      [ 0, 0, 1, 1 ], [ 0, 1, 0, 1 ],
      [ 0, 0, 1, 2 ], [ 1, 2, 0, 0 ],
      [ 0, 1, 0, 2 ], [ 1, 0, 2, 0 ],
      [ 0, 1, 2, 3 ]
    ];

    let mut prev_color_endpoint_index = 0;
    let mut prev_color_selector_index = 0;
    let mut prev_alpha_endpoint_index = 0;
    let mut prev_alpha_selector_index = 0;

    let block_size = 16;
    let pitch = block_x as usize * block_size;

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

          if chunk_encoding_bits == 1 {
            chunk_encoding_bits = tables.chunk_encoding.next(codec).context("read chunk encoding bits")?;
            chunk_encoding_bits |= 512;
          }

          let chunk_encoding_index = chunk_encoding_bits as usize & 7;
          chunk_encoding_bits >>= 3;

          let tiles_count = COUNT_TILES[chunk_encoding_index];
          let tiles = &TILES[chunk_encoding_index];

          // let mut dd = vec![];
          for i in 0..tiles_count {
            let delta = tables.alpha_endpoint_delta.next(codec).context("read alpha_endpoint_delta")? as usize;
            prev_alpha_endpoint_index += delta;
            prev_alpha_endpoint_index %= tables.alpha_endpoints.len();
            alpha_endpoints[i] = tables.alpha_endpoints[prev_alpha_endpoint_index];
            // dd.push((delta, prev_alpha_endpoint_index, (alpha_endpoints[i].0 as u16, alpha_endpoints[i].1 as u16)));
          }

          for i in 0..tiles_count {
            let delta = tables.color_endpoint_delta.next(codec).context("read color_endpoint_delta")? as usize;
            prev_color_endpoint_index += delta;
            prev_color_endpoint_index %= tables.color_endpoints.len();
            color_endpoints[i] = tables.color_endpoints[prev_color_endpoint_index];
            // dd.push((delta, prev_color_endpoint_index, color_endpoints[i]));
          }
          // println!("dd: {:x?}", dd);
          // println!("tile: {:x?}", tiles);
          for (i, &tile) in tiles.iter().enumerate() {
            let delta0 = tables.alpha_selector_delta.next(codec).context("read alpha_selector_delta")? as usize;
            prev_alpha_selector_index += delta0;
            prev_alpha_selector_index %= tables.alpha_selectors.len();
            let alpha_selector = tables.alpha_selectors[prev_alpha_selector_index];

            let delta1 = tables.color_selector_delta.next(codec).context("read color_selector_delta")? as usize;
            prev_color_selector_index += delta1;
            prev_color_selector_index %= tables.color_selectors.len();
            let color_selector = tables.color_selectors[prev_color_selector_index];

            // println!("{:x?}", (delta0, delta1, prev_alpha_selector_index, prev_color_selector_index, tables.color_selectors[prev_color_selector_index]));
            if !skip_x && !skip_y {
              if i % 2 == 0 {
                let pos = (y * 2 + i / 2) * pitch + x * block_size * 2;
                println!("seek {}x{} + {} => {:x}", x, y, i, pos);
                cursor.seek(std::io::SeekFrom::Start(pos as _)).expect("seek");
              }
              cursor.write(&[alpha_endpoints[tile].0, alpha_endpoints[tile].1]).expect("write alpha_endpoints");
              cursor.write(&alpha_selector).expect("write alpha_endpoints");
              cursor.write(&[
                color_endpoints[tile].0 as u8,
                (color_endpoints[tile].0 >> 8) as u8,
                color_endpoints[tile].1 as u8,
                (color_endpoints[tile].1 >> 8) as u8]).expect("write color_endpoints");
              cursor.write(&color_selector).expect("write color_selector");
            }
          }
        }
      }
    }
    if !codec.is_complete() { bail!("extra bytes in codec") }
    Ok(result)
  }
}
