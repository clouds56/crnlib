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
    if self.index + n > self.buffer.len() {
      self.buffer[self.index..].load_be::<u64>() << (self.index + n - self.buffer.len())
    } else {
      self.buffer[self.index..self.index+n].load_be()
    }
  }
  pub fn read_bits(&mut self, n: usize) -> Result<u64, Error> {
    assert!(n <= 64);
    if self.index + n > self.buffer.len() {
      bail!("read out of index {} < {}", self.index+n, self.buffer.len());
    }
    if n == 0 { return Ok(0) }
    let result = self.buffer[self.index..self.index+n].load_be();
    self.index += n;
    Ok(result)
  }
  pub fn skip_bits(&mut self, n: usize) {
    self.index += n;
  }
  pub fn current(&self) -> usize {
    self.index
  }
  pub fn len(&self) -> usize {
    self.buffer.len()
  }
  pub fn is_complete(&self) -> bool {
    self.index + 7 >= self.buffer.len() && self.index <= self.buffer.len()
  }

  pub fn get_huffman(&mut self) -> Result<Huffman<u32>, Error> {
    let symbol_count = self.read_bits(Huffman::<()>::MAX_SYMBOL_COUNT_BIT)? as u32;
    // println!("construct huffman tree with {} symbols", symbol_count);
    if symbol_count == 0 {
      return Huffman::new(BTreeMap::new())
    }
    let mut tmp_symbol_depth = BTreeMap::new();
    let tmp_symbol_count = self.read_bits(5)? as usize;
    ensure!(tmp_symbol_count < Key::SHUFFLE.len(), anyhow!("symbol_size_count"));
    for i in 0..tmp_symbol_count {
      let value = self.read_bits(3)? as usize;
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
      let (len, d) = match key.next(self).context("get key content")? {
        Depth(d) => (1, d),
        ShortZero => (self.read_bits(3)? + 3, 0),
        LongZero => (self.read_bits(7)? + 11, 0),
        ShortRepeat => (self.read_bits(2)? + 3, last.ok_or_else(|| anyhow!("short repeat no last"))?),
        LongRepeat => (self.read_bits(6)? + 7, last.ok_or_else(|| anyhow!("long repeat no last"))?),
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
  assert_eq!(codec.read_bits(3).unwrap(), 0b110);
  assert_eq!(codec.index, 3);
  assert_eq!(codec.read_bits(17).unwrap(), 0b1010_0110_1101_1101);
  assert_eq!(codec.index, 20);
  assert_eq!(codec.read_bits(0).unwrap(), 0);
  assert_eq!(codec.index, 20);

  assert_eq!(Huffman::<()>::MAX_SYMBOL_COUNT, 1 << (Huffman::<()>::MAX_SYMBOL_COUNT_BIT - 1));
}

pub struct Huffman<T> {
  depth_count: [usize; Key::MAX_DEPTH+1],
  /// [0, 1, 3, 7, 15, 32]
  /// for depth of i, the range of encoded is depth_bound[i-1]*2..depth_bound[i]
  /// 1: 0..1 => 0b0
  /// 2: 2..3 => 0b10
  /// 3: 6..7 => 0b110
  /// 4: 14..15 => 0b1110
  /// 5: 30..32 => 0b11110, 0b11111
  // depth_bound: [u32; Key::MAX_DEPTH+1],
  symbol_depth: BTreeMap<T, usize>,
  symbols: BTreeMap<T, u32>,
  symbol_rev: BTreeMap<(usize, u32), T>,
  max_depth: usize,
}

impl<T: std::fmt::Debug> std::fmt::Debug for Huffman<T> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Huffman")
      .field("symbol_count", &self.symbols.len())
      .field("max_depth", &self.max_depth)
      .field("symbol_depth", &self.symbol_depth)
      .field("depth_count", &self.depth_count)
      .finish()
  }
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
    ensure!(
      1<<max_depth == depth_bound[max_depth] || (max_depth <= 1 && depth_bound[max_depth] == max_depth as u32),
      "depth_bound error: {:?} {:?}", depth_count, depth_bound);
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
      depth_count, symbol_depth, max_depth,
      symbols, symbol_rev,
    })
  }

  pub fn next(&self, codec: &mut Codec<'_>) -> Result<T, Error> {
    ensure!(codec.current() < codec.len(), "stream end {} >= {}", codec.current(), codec.len());
    let k = codec.look_bits(self.max_depth) as u32;
    for i in 1..=self.max_depth {
      let t = k >> (self.max_depth - i);
      if let Some(sym) = self.symbol_rev.get(&(i, t)) {
        codec.index += i;
        return Ok(*sym)
      }
    }
    bail!("incomplete huffman tree no match");
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

#[test]
fn test_huffman() {
  let input = [0b0100_0000u8];
  let mut codec = Codec::new(&input);
  let huffman = Huffman::new(BTreeMap::<bool,_>::new()).expect("zero huffman");
  assert!(huffman.next(&mut codec).is_err());

  let mut codec = Codec::new(&input);
  let mut depth = BTreeMap::new();
  depth.insert(0xff, 1);
  let huffman = Huffman::new(depth).expect("zero huffman");
  assert_eq!(huffman.next(&mut codec).unwrap(), 0xff);
  assert!(huffman.next(&mut codec).is_err());


  let mut codec = Codec::new(&input);
  let mut depth = BTreeMap::new();
  depth.insert(0x01, 1);
  depth.insert(0xff, 1);
  let huffman = Huffman::new(depth).expect("zero huffman");
  assert_eq!(huffman.next(&mut codec).unwrap(), 0x01);
  assert_eq!(huffman.next(&mut codec).unwrap(), 0xff);
}
