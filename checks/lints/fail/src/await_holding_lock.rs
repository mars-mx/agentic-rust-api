pub async fn f(m: &std::sync::Mutex<u8>) -> u8 {
    let guard = m.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    tokio::task::yield_now().await;
    *guard
}
