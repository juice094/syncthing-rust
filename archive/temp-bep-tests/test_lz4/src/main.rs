fn main() {
    let src = b"hello world hello world hello world";
    let compressed = lz4::block::compress(src, None, false).unwrap();
    println!("compressed: {:02x?}", compressed);
    let decompressed = lz4::block::decompress(&compressed, Some(src.len() as i32)).unwrap();
    println!("decompressed: {:?}", std::str::from_utf8(&decompressed));
}
