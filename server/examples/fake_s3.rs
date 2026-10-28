//! A local stand-in for R2 for `just dev-server`/`dev-client`: `fake-s3 [ADDR] [DIR]`.

use pulse_server::testing::fake_s3::{FAKE_BUCKET, FAKE_KEY_ID, FAKE_SECRET, FakeS3, FakeStore};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:7895".into());
    let dir = args.next().unwrap_or_else(|| "data/fake-s3".into());
    let fake = FakeS3::start_on(
        addr.parse().expect("ADDR is host:port"),
        FakeStore::Dir(dir.clone().into()),
    )
    .await;
    println!("fake S3 (not secure, dev only)");
    println!("  endpoint: {}", fake.endpoint());
    println!("  bucket:   {FAKE_BUCKET}");
    println!("  key id:   {FAKE_KEY_ID}");
    println!("  secret:   {FAKE_SECRET}");
    println!("  region:   auto (path style)");
    println!("  objects:  {dir}");
    std::future::pending::<()>().await;
}
