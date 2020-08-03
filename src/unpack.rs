use anyhow::*;
use crate::{Tables, codec::Codec};

pub struct Dxt5;

pub trait Unpack {
  fn unpack(tables: &Tables, codec: &mut Codec, width: u16, height: u16, face: u8) -> Result<Vec<u16>, Error>;
}

impl Unpack for Dxt5 {
  fn unpack(tables: &Tables, codec: &mut Codec, width: u16, height: u16, face: u8) -> Result<Vec<u16>, Error> {
    let block_x = (width + 3) / 4;
    let block_y = (height + 3) / 4;
    let chunk_x = (block_x + 1) / 2;
    let chunk_y = (block_y + 1) / 2;

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

    let mut result = Vec::new();

    for _f in 0..face {
      // let mut row = Vec::new();
      for y in 0..chunk_y {
        let xrange: Box<dyn Iterator<Item=_>> = if y & 1 == 1 { Box::new((0..chunk_x).rev()) } else { Box::new(0..chunk_x) };
        for x in xrange {
          let mut color_endpoints = [0; 4];
          let mut alpha_endpoints = [0; 4];

          if chunk_encoding_bits == 1 {
            chunk_encoding_bits = tables.chunk_encoding.next(codec).context("read chunk encoding bits")?;
            chunk_encoding_bits |= 512;
          }

          let chunk_encoding_index = chunk_encoding_bits as usize & 7;
          chunk_encoding_bits >>= 3;

          let tiles_count = COUNT_TILES[chunk_encoding_index];
          let tiles = &TILES[chunk_encoding_index];

          for i in 0..tiles_count {
            let delta = tables.alpha_endpoint_delta.next(codec).context("read alpha_endpoint_delta")? as usize;
            prev_alpha_endpoint_index += delta;
            prev_alpha_endpoint_index %= tables.alpha_endpoints.len();
            alpha_endpoints[i] = tables.alpha_endpoints[prev_alpha_endpoint_index];
          }

          for i in 0..tiles_count {
            let delta = tables.color_endpoint_delta.next(codec).context("read color_endpoint_delta")? as usize;
            prev_color_endpoint_index += delta;
            prev_color_endpoint_index %= tables.color_endpoints.len();
            color_endpoints[i] = tables.color_endpoints[prev_color_endpoint_index];
          }

          for &tile in tiles {
            let delta = tables.alpha_selector_delta.next(codec).context("read alpha_selector_delta")? as usize;
            prev_alpha_selector_index += delta;
            prev_alpha_selector_index %= tables.alpha_selectors.len();
            let alpha_selector = tables.alpha_selectors[prev_alpha_selector_index];

            let delta = tables.color_selector_delta.next(codec).context("read color_selector_delta")? as usize;
            prev_color_selector_index += delta;
            prev_color_selector_index %= tables.alpha_selectors.len();
            let color_selector = tables.color_selectors[prev_color_selector_index];

            result.push(alpha_endpoints[tile]);
            result.push(alpha_selector.0);
            result.push(alpha_selector.1);
            result.push(alpha_selector.2);
            result.push(color_endpoints[tile] as u16);
            result.push((color_endpoints[tile] >> 16) as u16);
            result.push(color_selector as u16);
            result.push((color_selector >> 16) as u16);
          }
          if block_x & 1 == 1 && x == (chunk_x - 1) {
            result.truncate(result.len() - 8);
          }
        }

        if y == (chunk_y - 1) && block_y & 1 == 1 {
          result.truncate(result.len() - 16);
        }
      }
    }
    if !codec.is_complete() { bail!("extra bytes in codec") }
    Ok(result)
  }
}
