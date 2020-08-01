use std::collections::BTreeMap;
use bitvec::{slice::BitSlice, order::Msb0, fields::BitField};
use anyhow::*;

pub struct Codec<'a> {
  buffer: &'a BitSlice<Msb0, u8>,
  index: usize,
}

impl Codec<'_> {
  pub fn new<'a>(input: &'a [u8]) -> Codec<'a> {
    Codec { buffer: BitSlice::from_slice(input), index: 0 }
  }
  pub fn look_bits(&self, n: usize) -> u64 {
    assert!(n <= 64);
    if n == 0 { return 0 }
    self.buffer[self.index..self.index+n].load_be()
  }
  pub fn read_bits(&mut self, n: usize) -> u64 {
    assert!(n <= 64);
    let result = self.look_bits(n);
    self.index += n;
    result
  }
  pub fn skip_bits(&mut self, n: usize) {
    self.index += n;
  }
  pub fn current(&self) -> usize {
    self.index
  }

  pub fn decode(&mut self) -> Result<Huffman<u32>, Error> {
    let symbol_count = self.read_bits(Huffman::<()>::MAX_SYMBOL_COUNT_BIT) as u32;
    let mut tmp_symbol_depth = BTreeMap::new();
    let tmp_symbol_count = self.read_bits(5) as usize;
    ensure!(tmp_symbol_count < Key::SHUFFLE.len(), anyhow!("symbol_size_count"));
    for i in 0..tmp_symbol_count {
      let value = self.read_bits(3) as usize;
      if value != 0 {
        tmp_symbol_depth.insert(Key::SHUFFLE[i], value);
      }
    }
    let key = Huffman::new(tmp_symbol_depth).context("get key huffman")?;
    // println!("tmp_symbol_depth: {:?}", key);
    let mut symbol_depth = BTreeMap::new();
    let mut i = 0;
    let mut last = None;
    while i < symbol_count {
      let (len, d) = match key.next(self) {
        Depth(d) => (1, d),
        ShortZero => (self.read_bits(3) + 3, 0),
        LongZero => (self.read_bits(7) + 11, 0),
        ShortRepeat => (self.read_bits(2) + 3, last.unwrap()),
        LongRepeat => (self.read_bits(6) + 7, last.unwrap()),
      };
      last = Some(d);
      for j in 0..len as u32 {
        if d != 0 {
          symbol_depth.insert(i+j, d);
        }
      }
      i += len as u32;
    }
    // println!("{:?}", symbol_depth);
    Huffman::new(symbol_depth)
  }
}

#[test]
fn test_read_bits() {
  let input = [0b1100_1010u8, 0b0110_1101, 0b1101_1001];
  let mut codec = Codec::new(&input);
  assert_eq!(codec.read_bits(3), 0b110);
  assert_eq!(codec.index, 3);
  assert_eq!(codec.read_bits(17), 0b1010_0110_1101_1101);
  assert_eq!(codec.index, 20);
  assert_eq!(codec.read_bits(0), 0);
  assert_eq!(codec.index, 20);

  assert_eq!(Huffman::<()>::MAX_SYMBOL_COUNT, 1 << (Huffman::<()>::MAX_SYMBOL_COUNT_BIT - 1));
}

#[derive(Debug)]
pub struct Huffman<T> {
  depth_count: [usize; Key::MAX_DEPTH+1],
  /// [0, 1, 3, 7, 15, 32]
  /// for depth of i, the range of encoded is depth_bound[i-1]*2..depth_bound[i]
  /// 1: 0..1 => 0b0
  /// 2: 2..3 => 0b10
  /// 3: 6..7 => 0b110
  /// 4: 14..15 => 0b1110
  /// 5: 30..32 => 0b11110, 0b11111
  depth_bound: [u32; Key::MAX_DEPTH+1],
  symbol_depth: BTreeMap<T, usize>,
  symbols: BTreeMap<T, u32>,
  symbol_rev: BTreeMap<(usize, u32), T>,
  max_depth: usize,
}

impl<T: Ord+Copy> Huffman<T> {
  pub fn new(symbol_depth: BTreeMap<T, usize>) -> Result<Self, Error> {
    let mut depth_count = [0; Key::MAX_DEPTH+1];
    for &depth in symbol_depth.values() {
      depth_count[depth] += 1;
    }
    let mut max_depth = 0;
    let mut depth_bound= [0; Key::MAX_DEPTH+1];
    let mut available = 0;
    for (depth, &n) in depth_count.iter().enumerate() {
      if n != 0 {
        max_depth = depth;
      }
      available <<= 1;
      if depth != 0 {
        available += n as u32;
      }
      depth_bound[depth] = available;
    }
    ensure!(1<<max_depth == depth_bound[max_depth], "depth_bound error: {:?} {:?}", depth_count, depth_bound);
    let mut depth_current = [0; Key::MAX_DEPTH+1];
    for i in 1..=Key::MAX_DEPTH {
      depth_current[i] = depth_bound[i-1]*2;
    }
    let symbols: BTreeMap<_, _> = symbol_depth.iter().filter_map(|(&key, &depth)| {
      if depth == 0 { return None }
      let result = depth_current[depth];
      depth_current[depth] += 1;
      Some((key, result))
    }).collect();
    let symbol_rev = symbols.iter().map(|(&k, &v)| ((symbol_depth[&k], v), k)).collect();
    Ok(Self {
      depth_count, symbol_depth,
      max_depth, depth_bound,
      symbols, symbol_rev,
    })
  }

  pub fn next(&self, codec: &mut Codec<'_>) -> T {
    let k = codec.look_bits(self.max_depth) as u32;
    for i in 1..=self.max_depth {
      let t = k >> (self.max_depth - i);
      if let Some(sym) = self.symbol_rev.get(&(i, t)) {
        codec.index += i;
        return *sym
      }
    }
    unreachable!("complete huffman tree mut match");
  }
}

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub enum Key {
  Depth(usize),
  ShortZero /* 17 */, LongZero /* 18 */,
  ShortRepeat /* 19 */, LongRepeat /* 20 */,
}
use Key::*;

impl Key {
  pub const MAX_DEPTH: usize = 16;
  pub const SHUFFLE: [Key; Self::MAX_DEPTH+5] = [
    ShortZero, LongZero, ShortRepeat, LongRepeat,
    Depth(0), Depth(8), Depth(7), Depth(9),
    Depth(6), Depth(10), Depth(5), Depth(11),
    Depth(4), Depth(12), Depth(3), Depth(13),
    Depth(2), Depth(14), Depth(1), Depth(15), Depth(16)];
}

impl<T> Huffman<T> {
  pub const MAX_SYMBOL_COUNT: usize = 8192;
  pub const MAX_SYMBOL_COUNT_BIT: usize = 14;
}
