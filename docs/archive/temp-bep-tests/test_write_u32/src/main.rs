#[tokio::main]
async fn main() {
    use tokio::io::AsyncWriteExt;
    let mut buf = Vec::new();
    buf.write_u32(0x2EA7D90B).await.unwrap();
    println!("write_u32 output: {:02x?}", buf);
}
