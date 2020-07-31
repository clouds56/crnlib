use std::collections::BTreeMap;
use anyhow::*;

pub struct Codec<'a> {
  buffer: &'a [u8],
  current_byte: usize,
  bits: u8,
  remain: usize,
}

impl Codec<'_> {
  pub fn new<'a>(input: &'a [u8]) -> Codec<'a> {
    Codec { buffer: input, current_byte: 0, bits: 0, remain: 0 }
  }
  fn read_bits(&mut self, n: usize) -> u64 {
    assert!(n <= 64);
    let mut bit_count = self.remain;
    let mut bits = self.bits as u128;
    while n > bit_count {
      bits <<= 8;
      bits |= self.buffer[self.current_byte] as u128;
      // bits |= self.buffer.get(self.current_byte).cloned().unwrap_or_default() as u128;
      self.current_byte += 1;
      bit_count += 8;
    }
    self.remain = bit_count - n;
    assert!(self.remain < 8);
    self.bits = bits as u8 & ((1 << self.remain) - 1);
    // println!("{}/{} {:b}", n, bit_count, bits);
    (bits >> self.remain) as u64
  }

  pub fn decode(&mut self) -> Result<Huffman<u32>, Error> {
    let symbol_count = self.read_bits(Huffman::<()>::MAX_SYMBOL_COUNT_BIT) as usize;
    let mut tmp_symbol_depth = BTreeMap::new();
    let tmp_symbol_count = self.read_bits(5) as usize;
    ensure!(tmp_symbol_count < Huffman::<()>::TRANS_SYMBOL_IDX.len(), anyhow!("symbol_size_count"));
    for i in 0..tmp_symbol_count {
      let value = self.read_bits(3) as usize;
      if value != 0 {
        tmp_symbol_depth.insert(Huffman::<()>::TRANS_SYMBOL_IDX[i], value);
      }
    }
    println!("tmp_symbol_depth: {:?}", Huffman::new(tmp_symbol_depth));
    let symbol_depth = BTreeMap::new();
    Ok(Huffman::new(symbol_depth))
  }
}

#[test]
fn test_read_bits() {
  let input = [0b1100_1010u8, 0b0110_1101, 0b1101_1001];
  let mut codec = Codec::new(&input);
  assert_eq!(codec.read_bits(3), 0b110);
  assert_eq!((codec.remain, codec.bits), (5, 0b1010));
  assert_eq!(codec.read_bits(17), 0b1010_0110_1101_1101);
  assert_eq!((codec.remain, codec.bits), (4, 0b1001));
  assert_eq!(codec.read_bits(0), 0);
  assert_eq!((codec.remain, codec.bits), (4, 0b1001));

  assert_eq!(Huffman::<()>::MAX_SYMBOL_COUNT, 1 << (Huffman::<()>::MAX_SYMBOL_COUNT_BIT - 1));
}


#[derive(Debug)]
pub struct Huffman<T> {
  depth_count: [usize; Huffman::<()>::MAX_SYMBOL_DEPTH+1],
  /// [0, 1, 3, 7, 15, 32]
  /// for depth of i, the range of encoded is depth_bound[i-1]*2..depth_bound[i]
  /// 1: 0..1 => 0b0
  /// 2: 2..3 => 0b10
  /// 3: 6..7 => 0b110
  /// 4: 14..15 => 0b1110
  /// 5: 30..32 => 0b11110, 0b11111
  depth_bound: [u32; Huffman::<()>::MAX_SYMBOL_DEPTH+1],
  symbol_depth: BTreeMap<T, usize>,
  symbols: BTreeMap<T, u32>,
  symbol_rev: BTreeMap<u32, T>,
  max_depth: usize,
}

impl<T: Ord+Copy> Huffman<T> {
  pub fn new(symbol_depth: BTreeMap<T, usize>) -> Self {
    let mut depth_count = [0; 17];
    for &depth in symbol_depth.values() {
      depth_count[depth] += 1;
    }
    let mut max_depth = 0;
    let mut depth_bound= [0; 17];
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
    // assert_eq!(1<<max_depth, depth_bound[max_depth]);
    let mut depth_current = [0; 17];
    for i in 1..17 {
      depth_current[i] = depth_bound[i-1]*2;
    }
    let symbols: BTreeMap<_, _> = symbol_depth.iter().map(|(&key, &depth)| {
      let result = depth_current[depth];
      depth_current[depth] += 1;
      (key, result)
    }).collect();
    let symbol_rev = symbols.iter().map(|(&k, &v)| (v, k)).collect();
    Self {
      depth_count, symbol_depth,
      max_depth, depth_bound,
      symbols, symbol_rev,
    }
  }
}

pub struct HuffmanCodec<'a, T> {
  huffman: &'a Huffman<T>,
  codec: &'a Codec<'a>,
}

impl<T> Iterator for HuffmanCodec<'_, T> {
  type Item = T; // 0..MAX_SYMBOL
  fn next(&mut self) -> Option<Self::Item> {
    None
  }
}

impl<T> Huffman<T> {
  pub const MAX_SYMBOL_COUNT: usize = 8192;
  pub const MAX_SYMBOL_COUNT_BIT: usize = 14;
  pub const MAX_SYMBOL_DEPTH: usize = 16;
  pub const TRANS_SYMBOL_IDX: [usize; Huffman::<()>::MAX_SYMBOL_DEPTH+5] = [
    17, 18, 19, 20,
    0, 8, 7, 9,
    6, 10, 5, 11,
    4, 12, 3, 13,
    2, 14, 1, 15, 16];
}
