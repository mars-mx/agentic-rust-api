pub async fn f() {
    let span = tracing::info_span!("x").entered();
    tokio::task::yield_now().await;
    drop(span);
}
