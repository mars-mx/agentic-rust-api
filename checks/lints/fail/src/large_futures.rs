pub async fn big() -> usize {
    let buffer = [0u8; 20_000];
    tokio::task::yield_now().await;
    buffer.iter().filter(|byte| **byte == 0).count()
}

pub async fn f() -> usize {
    big().await
}
