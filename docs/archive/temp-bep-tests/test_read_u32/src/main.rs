#[tokio::main]
async fn main() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = Vec::new();
    buf.write_u32(0x2EA7D90B).await.unwrap();
    buf.write_u16(0x0102).await.unwrap();
    println!("write output: {:02x?}", buf);
    
    let mut cursor = std::io::Cursor::new(buf);
    let r32 = cursor.read_u32().await.unwrap();
    let r16 = cursor.read_u16().await.unwrap();
    println!("read_u32: 0x{:08X}", r32);
    println!("read_u16: 0x{:04X}", r16);
}
