async fn f(p: PgPool) { sqlx::migrate!().run(&p).await; }
