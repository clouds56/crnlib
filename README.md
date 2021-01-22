crnlib
========
This is a port from [crunch/crnlib](https://github.com/BinomialLLC/crunch), the license could be found at the end of file.

Feel free to open a issue about usage and/or features to make it better to use. Now (Jan 2021) it still works well for my specific usage.

Usage
========
```rust
use std::io::prelude::*;
let sample = "samples/test.crn";
let mut file = std::fs::File::open(sample).expect("open sample crn file");
let mut buffer = Vec::new();

let header = Header::parse(&buffer).expect("parse");

let tables = header.get_table(&buffer).expect("read table");
let level0 = header.unpack_level(&tables, &buffer, 0).expect("unpack");

// level0 contains DXT encoded image content, which could be read by image
use image::ImageDecoder;
let (width0, height0) = header.get_level_info(0).expect("get level info");
let variant = match header.format {
  Format::Dxt1 => image::dxt::DXTVariant::DXT1,
  Format::Dxt3 => image::dxt::DXTVariant::DXT3,
  Format::Dxt5 => image::dxt::DXTVariant::DXT5,
  format => unimplemented!("image does not support format {:?}", format),
};
let decoder = image::dxt::DxtDecoder::new(std::io::Cursor::new(&level0), width0 as u32, height0 as u32, variant).expect("new image");
let mut raw = vec![0; decoder.total_bytes() as usize];
let color_type = decoder.color_type();
decoder.read_image(&mut raw).expect("decode dxt");
let f = std::fs::File::create(std::path::Path::new(sample).with_extension("tga")).expect("create sample tga file");
let encoder = image::tga::TgaEncoder::new(f);
encoder.encode(&raw, width0 as u32, height0 as u32, color_type).expect("encode tga");
```

Document of Table
========
* Any table contains 2 huffman tree
    * 14 bit of max_symbol_count of second tree
    * "symbol_count" of first temporary tree, the symbol was reordered as `[ShortZero, LongZero, ShortRepeat, LongRepeat, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15, 16]`.
    * temporary one has Key `0..=16` as well as `ShortZero, LongZero, ShortRepeat, LongRepeat` which come with a parameter with `3, 7, 2, 6` bits.
    * the depth array (of length max_symbol_count) of second one is encoded using first one.
* A huffman tree is stored using symbol_depth.
    * symbol_depth means length of code in bits.
    * the Code is assign to Key from small depth to large, then follow the ord of Key.

License of Crunch
========
```
crunch/crnlib uses a modified ZLIB license. Specifically, it's the same as zlib except that
public credits for using the library are *required*.

Copyright (c) 2010-2016 Richard Geldreich, Jr. All rights reserved.

This software is provided 'as-is', without any express or implied
warranty.  In no event will the authors be held liable for any damages
arising from the use of this software.

Permission is granted to anyone to use this software for any purpose,
including commercial applications, and to alter it and redistribute it
freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not
claim that you wrote the original software.

2. If you use this software in a product, this acknowledgment in the product
documentation or credits is required:

"Crunch Library Copyright (c) 2010-2016 Richard Geldreich, Jr."

3. Altered source versions must be plainly marked as such, and must not be
misrepresented as being the original software.

4. This notice may not be removed or altered from any source distribution.
```
